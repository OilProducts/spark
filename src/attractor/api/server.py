from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
import copy
import json
import logging
import threading
import uuid
import os
import re
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

from fastapi import APIRouter, FastAPI, HTTPException, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, StreamingResponse
from pydantic import BaseModel

from attractor.dsl import (
    DotParseError,
    Diagnostic,
    DiagnosticSeverity,
    format_dot,
    parse_dot,
)
from attractor.engine import (
    Checkpoint,
    Context,
    Outcome,
    OutcomeStatus,
    PipelineExecutor,
    load_checkpoint,
    save_checkpoint,
)
from attractor.graphviz_export import export_graphviz_artifact
from attractor.graph_prep import (
    DEFAULT_MAX_RETRIES_KEY,
    build_transform_pipeline as _build_graph_transform_pipeline,
    canonicalize_graph_source as _canonicalize_graph_source,
    prepare_graph as _prepare_graph_impl,
    resolve_default_max_retries_value,
)
from attractor.api.codex_backends import (
    CodexAppServerBackend,
    build_codergen_backend as _build_codergen_backend_impl,
)
from attractor.validation_preview import diagnostic_payload as _diagnostic_payload_impl
from attractor.api.flow_sources import (
    ensure_flows_dir as _ensure_flows_dir_impl,
    flow_name_from_path as _flow_name_from_path_impl,
    inject_pipeline_goal as _inject_pipeline_goal_impl,
    load_flow_content as _load_flow_content_impl,
    resolve_flow_path as _resolve_flow_path_impl,
    semantic_signature as _semantic_signature_impl,
)
from attractor.api.run_records import (
    RunRecord,
    hydrate_run_record_from_log,
    normalize_run_status,
    run_matches_project_scope,
)
from attractor.api import pipeline_runs
from attractor.api.pipeline_runtime import (
    ActiveRun,
    BroadcastingRunner,
    ConnectionManager,
    ExecutionControl,
    HumanGateBroker,
    PipelineEventHub,
    RunListEventHub,
    RuntimeState,
    WebInterviewer,
)
from attractor.api.token_usage import TokenUsageBreakdown, estimate_model_cost
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import (
    ChildRunRequest,
    ChildRunResult,
    CodergenBackend,
    PIPELINE_RETRY_RUN_ID_CONTEXT_KEY,
)
from attractor.llm_runtime import (
    RUNTIME_LAUNCH_MODEL_KEY,
    RUNTIME_LAUNCH_PROVIDER_KEY,
    RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
    node_uses_llm_backend,
    resolve_effective_llm_provider,
)
from attractor.interviewer.base import Interviewer
from attractor.interviewer.models import Answer, AnswerValue, Question
from attractor.launch_context import normalize_launch_context
from attractor.api.runtime_paths import (
    AttractorRuntimePaths,
    resolve_runtime_paths as resolve_attractor_runtime_paths,
    validate_runtime_paths as validate_attractor_runtime_paths,
)
from spark_common.runtime_path import resolve_runtime_workspace_path
from spark_common.codex_runtime import build_codex_runtime_environment


attractor_app = FastAPI(title="Attractor API", docs_url="/docs", redoc_url=None, openapi_url="/openapi.json")
attractor_router = APIRouter()
LOGGER = logging.getLogger(__name__)
RUNTIME_PATHS_LOCK = threading.Lock()
RUNTIME_PATHS: AttractorRuntimePaths | None = None
REGISTERED_TRANSFORMS: List[object] = []
_REGISTERED_TRANSFORMS_LOCK = threading.Lock()
_UNSET = object()


def get_runtime_paths() -> AttractorRuntimePaths:
    with RUNTIME_PATHS_LOCK:
        if RUNTIME_PATHS is None:
            raise RuntimeError(
                "Attractor runtime paths are not configured. "
                "Inject them from spark.app or an explicit test fixture before serving requests."
            )
        return RUNTIME_PATHS


def configure_runtime_paths(
    *,
    runtime_dir: Path | str | None | object = _UNSET,
    runs_dir: Path | str | None | object = _UNSET,
    flows_dir: Path | str | None | object = _UNSET,
) -> AttractorRuntimePaths:
    global RUNTIME_PATHS
    with RUNTIME_PATHS_LOCK:
        current = RUNTIME_PATHS
    updated = resolve_attractor_runtime_paths(
        runtime_dir=current.runtime_dir if runtime_dir is _UNSET and current is not None else runtime_dir,
        runs_dir=current.runs_dir if runs_dir is _UNSET and current is not None else runs_dir,
        flows_dir=current.flows_dir if flows_dir is _UNSET and current is not None else flows_dir,
    )
    validate_attractor_runtime_paths(updated)
    with RUNTIME_PATHS_LOCK:
        RUNTIME_PATHS = updated
    return updated


def validate_runtime_paths() -> None:
    validate_attractor_runtime_paths(get_runtime_paths())


def register_transform(transform: object) -> None:
    with _REGISTERED_TRANSFORMS_LOCK:
        REGISTERED_TRANSFORMS.append(transform)


def clear_registered_transforms() -> None:
    with _REGISTERED_TRANSFORMS_LOCK:
        REGISTERED_TRANSFORMS.clear()


def _registered_transforms_snapshot() -> List[object]:
    with _REGISTERED_TRANSFORMS_LOCK:
        return list(REGISTERED_TRANSFORMS)


manager = ConnectionManager()
HUMAN_BROKER = HumanGateBroker()
EVENT_HUB = PipelineEventHub()
RUNS_EVENT_HUB = RunListEventHub()
RUNTIME = RuntimeState(last_completed_nodes=[])
ACTIVE_RUNS_LOCK = threading.Lock()
ACTIVE_RUNS: Dict[str, ActiveRun] = {}
RUN_EVENT_SEQUENCE_LOCK = threading.Lock()
RUN_EVENT_SEQUENCES: Dict[str, int] = {}
ATTRACTOR_RUNTIME_LOCK = threading.Lock()
ATTRACTOR_RUNTIME_INITIALIZED = False
ORPHANED_ACTIVE_STATUSES = {"running", "cancel_requested", "pause_requested"}
TERMINAL_RUN_STATUSES = {"completed", "failed", "validation_error", "paused", "canceled"}


RUN_HISTORY_LOCK = threading.Lock()
PIPELINE_LIFECYCLE_PHASES = ("PARSE", "TRANSFORM", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE")
DEFAULT_RUN_JOURNAL_PAGE_SIZE = 100
MAX_RUN_JOURNAL_PAGE_SIZE = 250


def _runs_root() -> Path:
    return pipeline_runs.runs_root(get_runtime_paths)


def _project_runs_dir(project_path: str) -> Optional[Path]:
    return pipeline_runs.project_runs_dir(get_runtime_paths, project_path)


def _iter_run_roots(*, project_path: Optional[str] = None) -> list[Path]:
    return pipeline_runs.iter_run_roots(get_runtime_paths, project_path=project_path)


def _find_run_root(run_id: str) -> Optional[Path]:
    return pipeline_runs.find_run_root(get_runtime_paths, run_id)


def _ensure_run_root_for_project(run_id: str, project_path: str) -> Path:
    return pipeline_runs.ensure_run_root_for_project(get_runtime_paths, run_id, project_path)


def _run_root(run_id: str) -> Path:
    return pipeline_runs.run_root(get_runtime_paths, run_id)


def _orphaned_run_terminal_state(status: str) -> tuple[str, str]:
    normalized = normalize_run_status(status)
    if normalized == "cancel_requested":
        return (
            "canceled",
            "Run was interrupted when the Attractor server stopped before cancellation completed.",
        )
    if normalized == "pause_requested":
        return (
            "failed",
            "Run was interrupted when the Attractor server stopped before pause completed.",
        )
    return (
        "failed",
        "Run was interrupted when the Attractor server stopped before completion.",
    )


def reconcile_orphaned_runs_on_startup() -> list[str]:
    reconciled: list[str] = []
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
    for run_root in _iter_run_roots():
        record = pipeline_runs.read_run_meta(run_root / "run.json")
        if record is None:
            continue
        status = normalize_run_status(record.status)
        if status not in ORPHANED_ACTIVE_STATUSES:
            continue
        if _get_active_run(record.run_id) is not None:
            continue
        final_status, last_error = _orphaned_run_terminal_state(status)
        _append_run_log(
            record.run_id,
            f"[{now} UTC] [System] Reconciled orphaned active run after server restart: {last_error}",
        )
        _record_run_end(
            record.run_id,
            record.working_directory or record.project_path,
            final_status,
            last_error,
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
        )
        reconciled.append(record.run_id)
    return reconciled


def initialize_attractor_runtime() -> list[str]:
    global ATTRACTOR_RUNTIME_INITIALIZED
    with ATTRACTOR_RUNTIME_LOCK:
        if ATTRACTOR_RUNTIME_INITIALIZED:
            return []
        reconciled_run_ids = reconcile_orphaned_runs_on_startup()
        ATTRACTOR_RUNTIME_INITIALIZED = True
    return reconciled_run_ids


def shutdown_attractor_runtime() -> None:
    global ATTRACTOR_RUNTIME_INITIALIZED
    with ATTRACTOR_RUNTIME_LOCK:
        ATTRACTOR_RUNTIME_INITIALIZED = False
    with ACTIVE_RUNS_LOCK:
        ACTIVE_RUNS.clear()
    with RUN_EVENT_SEQUENCE_LOCK:
        RUN_EVENT_SEQUENCES.clear()
    RUNS_EVENT_HUB.reset()


@asynccontextmanager
async def _attractor_lifespan(_: FastAPI):
    reconciled_run_ids = initialize_attractor_runtime()
    for run_id in reconciled_run_ids:
        await _publish_run_list_upsert(run_id)
    try:
        yield
    finally:
        shutdown_attractor_runtime()


def _resolve_start_node_id(graph) -> str:
    return pipeline_runs.resolve_start_node_id(graph)


def _graph_attr_context_seed(graph) -> Dict[str, object]:
    return pipeline_runs.graph_attr_context_seed(graph)


def _dot_attr_value(attrs: Dict[str, object], key: str, default: Optional[object] = None) -> Optional[object]:
    attr = attrs.get(key)
    if attr is None:
        return default
    value = getattr(attr, "value", default)
    if hasattr(value, "raw"):
        return value.raw
    return value


def _resolve_launch_model(graph, requested_model: Optional[str]) -> tuple[Optional[str], str]:
    selected_model = (requested_model or "").strip() or None
    if selected_model == "codex default (config/profile)":
        selected_model = None
    if selected_model:
        return selected_model, selected_model

    flow_default = _dot_attr_value(graph.graph_attrs, "ui_default_llm_model")
    if isinstance(flow_default, str):
        normalized_flow_default = flow_default.strip()
        if normalized_flow_default:
            return normalized_flow_default, normalized_flow_default

    return None, "codex default (config/profile)"


def _resolve_launch_provider(graph, requested_provider: Optional[str]) -> str:
    selected_provider = (requested_provider or "").strip().lower()
    if selected_provider:
        return selected_provider
    return "codex"


def _resolve_launch_reasoning_effort(graph, requested_reasoning_effort: Optional[str]) -> Optional[str]:
    del graph
    selected_reasoning_effort = (requested_reasoning_effort or "").strip().lower()
    if selected_reasoning_effort:
        return selected_reasoning_effort
    return None


class ProviderConfigurationError(ValueError):
    pass


def _provider_configuration_error(provider: str, missing_material: str) -> ProviderConfigurationError:
    return ProviderConfigurationError(
        f"Provider {provider} is not configured: missing {missing_material}."
    )


def _validate_provider_configuration(provider: str) -> None:
    normalized_provider = provider.strip().lower() or "codex"
    if normalized_provider == "openai":
        if not str(os.environ.get("OPENAI_API_KEY", "")).strip():
            raise _provider_configuration_error("openai", "OPENAI_API_KEY")
        return
    if normalized_provider == "anthropic":
        if not str(os.environ.get("ANTHROPIC_API_KEY", "")).strip():
            raise _provider_configuration_error("anthropic", "ANTHROPIC_API_KEY")
        return
    if normalized_provider == "gemini":
        if not (
            str(os.environ.get("GEMINI_API_KEY", "")).strip()
            or str(os.environ.get("GOOGLE_API_KEY", "")).strip()
        ):
            raise _provider_configuration_error("gemini", "GEMINI_API_KEY or GOOGLE_API_KEY")
        return
    if normalized_provider == "codex":
        env = build_codex_runtime_environment()
        codex_home_value = str(env.get("CODEX_HOME", "")).strip()
        if not codex_home_value:
            raise _provider_configuration_error("codex", "CODEX_HOME")
        codex_home = Path(codex_home_value).expanduser()
        if not codex_home.exists():
            raise _provider_configuration_error("codex", "CODEX_HOME directory")
        if not (codex_home / "auth.json").exists():
            raise _provider_configuration_error("codex", "CODEX_HOME/auth.json")
        return
    raise ProviderConfigurationError(
        "Unsupported llm_provider. Supported providers: codex, openai, anthropic, gemini."
    )


def _provider_preflight_context(
    graph,
    *,
    launch_context: Optional[dict[str, Any]],
    selected_provider: str,
    selected_model: Optional[str],
    selected_reasoning_effort: Optional[str],
) -> Context:
    context = Context(values=dict(launch_context or {}))
    context.apply_updates(_graph_attr_context_seed(graph))
    context.set(RUNTIME_LAUNCH_MODEL_KEY, selected_model or "")
    context.set(RUNTIME_LAUNCH_PROVIDER_KEY, selected_provider)
    context.set(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, selected_reasoning_effort or "")
    return context


def _validate_graph_provider_configuration(
    graph,
    *,
    launch_context: Optional[dict[str, Any]],
    selected_provider: str,
    selected_model: Optional[str],
    selected_reasoning_effort: Optional[str],
) -> None:
    context = _provider_preflight_context(
        graph,
        launch_context=launch_context,
        selected_provider=selected_provider,
        selected_model=selected_model,
        selected_reasoning_effort=selected_reasoning_effort,
    )
    providers = {
        resolve_effective_llm_provider(node.attrs, context, fallback_provider=selected_provider)
        for node in graph.nodes.values()
        if node_uses_llm_backend(node)
    }
    for provider in sorted(providers):
        _validate_provider_configuration(provider)


def _run_meta_path(run_id: str) -> Path:
    return pipeline_runs.run_meta_path(get_runtime_paths, run_id)


def _run_events_path(run_id: str) -> Path:
    return _run_root(run_id) / "events.jsonl"


def _run_root_exists(run_id: str) -> bool:
    return _find_run_root(run_id) is not None


def _read_persisted_run_events(run_id: str) -> list[dict[str, Any]]:
    events_path = _run_events_path(run_id)
    if not events_path.exists():
        return []

    events: list[dict[str, Any]] = []
    try:
        with events_path.open("r", encoding="utf-8") as handle:
            for line in handle:
                raw = line.strip()
                if not raw:
                    continue
                try:
                    payload = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if isinstance(payload, dict):
                    events.append(payload)
    except OSError:
        return []
    return events


def _persist_run_event(run_id: str, payload: dict[str, Any]) -> None:
    if not _run_root_exists(run_id):
        return

    events_path = _run_events_path(run_id)
    try:
        events_path.parent.mkdir(parents=True, exist_ok=True)
        with events_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, sort_keys=True) + "\n")
    except OSError:
        return


def _next_run_event_sequence(run_id: str) -> int:
    with RUN_EVENT_SEQUENCE_LOCK:
        current = RUN_EVENT_SEQUENCES.get(run_id)
        if current is None:
            current = 0
            for event in _read_persisted_run_events(run_id):
                sequence = event.get("sequence")
                if isinstance(sequence, int) and sequence > current:
                    current = sequence
        next_value = current + 1
        RUN_EVENT_SEQUENCES[run_id] = next_value
        return next_value


def _has_persisted_run_events(run_id: str) -> bool:
    events_path = _run_events_path(run_id)
    return events_path.exists() and events_path.stat().st_size > 0


def _normalize_run_journal_page_limit(limit: int | None) -> int:
    if limit is None:
        return DEFAULT_RUN_JOURNAL_PAGE_SIZE
    if limit <= 0:
        raise HTTPException(status_code=400, detail="limit must be greater than zero")
    return min(limit, MAX_RUN_JOURNAL_PAGE_SIZE)


def _normalize_before_sequence(value: int | None) -> int | None:
    if value is None:
        return None
    if value <= 0:
        raise HTTPException(status_code=400, detail="before_sequence must be greater than zero")
    return value


def _normalize_after_sequence(value: int | None) -> int | None:
    if value is None:
        return None
    if value < 0:
        raise HTTPException(status_code=400, detail="after_sequence must be zero or greater")
    return value


def _as_trimmed_string(value: Any) -> Optional[str]:
    if not isinstance(value, str):
        return None
    trimmed = value.strip()
    return trimmed or None


def _journal_source_scope(payload: dict[str, Any]) -> str:
    return "child" if payload.get("source_scope") == "child" else "root"


def _journal_source_parent_node_id(payload: dict[str, Any]) -> Optional[str]:
    return _as_trimmed_string(payload.get("source_parent_node_id"))


def _journal_source_flow_name(payload: dict[str, Any]) -> Optional[str]:
    return _as_trimmed_string(payload.get("source_flow_name"))


def _journal_source_prefix(
    *,
    source_scope: str,
    source_parent_node_id: Optional[str],
    source_flow_name: Optional[str],
) -> str:
    if source_scope != "child":
        return ""
    parent_label = source_parent_node_id or "parent node"
    if source_flow_name:
        return f"Child flow {source_flow_name} via {parent_label}: "
    return f"Child flow via {parent_label}: "


def _journal_node_id(payload: dict[str, Any]) -> Optional[str]:
    for candidate in (
        payload.get("node_id"),
        payload.get("node"),
        payload.get("name"),
        payload.get("stage"),
        payload.get("current_node"),
    ):
        node_id = _as_trimmed_string(candidate)
        if node_id:
            return node_id
    return None


def _journal_stage_index(payload: dict[str, Any]) -> Optional[int]:
    value = payload.get("index")
    return value if isinstance(value, int) else None


def _journal_question_id(payload: dict[str, Any]) -> Optional[str]:
    return _as_trimmed_string(payload.get("question_id"))


def _journal_kind(raw_type: str) -> str:
    if raw_type == "log":
        return "log"
    if raw_type in {"runtime", "state", "LLMContent"}:
        return raw_type
    if raw_type == "run_meta":
        return "metadata"
    if raw_type in {
        "PipelineStarted",
        "PipelineCompleted",
        "PipelineFailed",
        "PipelineRetryStarted",
        "PipelineRetryCompleted",
        "ChildRunStarted",
        "ChildRunCompleted",
        "lifecycle",
    }:
        return "lifecycle"
    if raw_type.startswith("Stage"):
        return "stage"
    if raw_type.startswith("Parallel"):
        return "parallel"
    if raw_type.startswith("Interview") or raw_type == "human_gate":
        return "interview"
    if raw_type == "CheckpointSaved":
        return "checkpoint"
    return "other"


def _journal_severity(raw_type: str, payload: dict[str, Any]) -> str:
    severity_value = payload.get("severity") if isinstance(payload.get("severity"), str) else payload.get("level")
    severity = _as_trimmed_string(severity_value)
    if severity:
        normalized = severity.lower()
        if normalized in {"info", "warning", "error"}:
            return normalized
    if raw_type == "log":
        message = str(payload.get("msg", "")).lower()
        if "error" in message or "fail" in message:
            return "error"
        if "warn" in message:
            return "warning"
        return "info"
    if raw_type in {"PipelineFailed", "StageFailed"}:
        return "error"
    if raw_type in {"StageRetrying", "InterviewTimeout"}:
        return "warning"
    if raw_type == "PipelineCompleted" and payload.get("outcome") == "failure":
        return "warning"
    if raw_type == "runtime":
        status = _as_trimmed_string(payload.get("status")) or ""
        if status in {"failed", "validation_error"}:
            return "error"
        if status in {"cancel_requested", "abort_requested", "canceled", "aborted"}:
            return "warning"
    return "info"


def _interview_outcome_provenance(raw_type: str, payload: dict[str, Any]) -> Optional[str]:
    raw_provenance = _as_trimmed_string(payload.get("outcome_provenance")) or _as_trimmed_string(payload.get("provenance"))
    if raw_provenance in {"accepted", "skipped", "timeout_default_applied", "timeout_no_default"}:
        return raw_provenance
    if raw_type == "InterviewCompleted":
        answer = _as_trimmed_string(payload.get("answer"))
        if not answer:
            return None
        return "skipped" if answer.lower() == "skipped" else "accepted"
    if raw_type == "InterviewTimeout":
        default_choice = (
            _as_trimmed_string(payload.get("default_choice_label"))
            or _as_trimmed_string(payload.get("default_choice_target"))
        )
        return "timeout_default_applied" if default_choice else "timeout_no_default"
    return None


def _interview_default_choice_label(payload: dict[str, Any]) -> Optional[str]:
    return (
        _as_trimmed_string(payload.get("default_choice_label"))
        or _as_trimmed_string(payload.get("default_choice_target"))
    )


def _journal_summary(
    *,
    raw_type: str,
    payload: dict[str, Any],
    node_id: Optional[str],
    source_scope: str,
    source_parent_node_id: Optional[str],
    source_flow_name: Optional[str],
) -> str:
    source_prefix = _journal_source_prefix(
        source_scope=source_scope,
        source_parent_node_id=source_parent_node_id,
        source_flow_name=source_flow_name,
    )
    if raw_type == "log":
        return str(payload.get("msg", "")).strip() or "Log entry"
    if raw_type == "lifecycle":
        phase = _as_trimmed_string(payload.get("phase"))
        return f"{source_prefix}Lifecycle phase: {phase}" if phase else f"{source_prefix}Lifecycle event"
    if raw_type == "runtime":
        status = _as_trimmed_string(payload.get("status"))
        outcome = _as_trimmed_string(payload.get("outcome"))
        reason_message = _as_trimmed_string(payload.get("outcome_reason_message"))
        reason_code = _as_trimmed_string(payload.get("outcome_reason_code"))
        if status:
            summary = f"{source_prefix}Run status: {status}"
            if outcome:
                summary = f"{summary} ({outcome})"
            reason = reason_message or reason_code
            return f"{summary}: {reason}" if reason else summary
        return f"{source_prefix}Run status updated"
    if raw_type == "state":
        state_node = _as_trimmed_string(payload.get("node")) or node_id or "unknown"
        state_status = _as_trimmed_string(payload.get("status"))
        return f"{source_prefix}Node {state_node} status: {state_status or 'updated'}"
    if raw_type == "run_meta":
        flow_name = _as_trimmed_string(payload.get("flow_name"))
        return f"{source_prefix}Run metadata captured for {flow_name}" if flow_name else f"{source_prefix}Run metadata captured"
    if raw_type == "PipelineStarted":
        return f"{source_prefix}Pipeline started at {node_id or 'start'}"
    if raw_type == "PipelineCompleted":
        outcome = _as_trimmed_string(payload.get("outcome"))
        reason_code = _as_trimmed_string(payload.get("outcome_reason_code"))
        reason_message = _as_trimmed_string(payload.get("outcome_reason_message"))
        if outcome == "failure":
            if reason_message:
                return f"{source_prefix}Pipeline completed at {node_id or 'exit'} (failure: {reason_message})"
            if reason_code:
                return f"{source_prefix}Pipeline completed at {node_id or 'exit'} (failure: {reason_code})"
            return f"{source_prefix}Pipeline completed at {node_id or 'exit'} (failure)"
        return (
            f"{source_prefix}Pipeline completed at {node_id or 'exit'} ({outcome})"
            if outcome
            else f"{source_prefix}Pipeline completed at {node_id or 'exit'}"
        )
    if raw_type == "PipelineFailed":
        error = _as_trimmed_string(payload.get("error"))
        return f"{source_prefix}Pipeline failed: {error}" if error else f"{source_prefix}Pipeline failed"
    if raw_type == "PipelineRetryStarted":
        retry_node = _as_trimmed_string(payload.get("current_node")) or node_id or "checkpoint"
        return f"{source_prefix}Retry started from {retry_node}"
    if raw_type == "PipelineRetryCompleted":
        retry_status = _as_trimmed_string(payload.get("status"))
        return (
            f"{source_prefix}Retry completed ({retry_status})"
            if retry_status
            else f"{source_prefix}Retry completed"
        )
    if raw_type == "ChildRunStarted":
        child_run_id = _as_trimmed_string(payload.get("child_run_id"))
        child_flow_name = _as_trimmed_string(payload.get("child_flow_name"))
        label = child_flow_name or child_run_id or "child run"
        return f"{source_prefix}Child run started: {label}"
    if raw_type == "ChildRunCompleted":
        child_run_id = _as_trimmed_string(payload.get("child_run_id"))
        child_flow_name = _as_trimmed_string(payload.get("child_flow_name"))
        status = _as_trimmed_string(payload.get("status"))
        label = child_flow_name or child_run_id or "child run"
        return f"{source_prefix}Child run completed: {label} ({status})" if status else f"{source_prefix}Child run completed: {label}"
    if raw_type == "StageStarted":
        return f"{source_prefix}Stage {node_id or 'unknown'} started"
    if raw_type == "StageCompleted":
        outcome = _as_trimmed_string(payload.get("outcome"))
        return (
            f"{source_prefix}Stage {node_id or 'unknown'} completed ({outcome})"
            if outcome
            else f"{source_prefix}Stage {node_id or 'unknown'} completed"
        )
    if raw_type == "StageFailed":
        error = _as_trimmed_string(payload.get("error"))
        return (
            f"{source_prefix}Stage {node_id or 'unknown'} failed: {error}"
            if error
            else f"{source_prefix}Stage {node_id or 'unknown'} failed"
        )
    if raw_type == "StageRetrying":
        attempt = payload.get("attempt") if isinstance(payload.get("attempt"), int) else None
        return (
            f"{source_prefix}Stage {node_id or 'unknown'} retrying (attempt {attempt})"
            if attempt is not None
            else f"{source_prefix}Stage {node_id or 'unknown'} retrying"
        )
    if raw_type == "ParallelStarted":
        branch_count = payload.get("branch_count") if isinstance(payload.get("branch_count"), int) else None
        return (
            f"{source_prefix}Parallel fan-out started ({branch_count} branches)"
            if branch_count is not None
            else f"{source_prefix}Parallel fan-out started"
        )
    if raw_type == "ParallelBranchStarted":
        branch_name = _as_trimmed_string(payload.get("branch")) or node_id or "unknown"
        return f"{source_prefix}Parallel branch {branch_name} started"
    if raw_type == "ParallelBranchCompleted":
        branch_name = _as_trimmed_string(payload.get("branch")) or node_id or "unknown"
        success = "success" if payload.get("success") is True else "failed" if payload.get("success") is False else None
        return (
            f"{source_prefix}Parallel branch {branch_name} completed ({success})"
            if success
            else f"{source_prefix}Parallel branch {branch_name} completed"
        )
    if raw_type == "ParallelCompleted":
        success_count = payload.get("success_count") if isinstance(payload.get("success_count"), int) else None
        failure_count = payload.get("failure_count") if isinstance(payload.get("failure_count"), int) else None
        if success_count is not None and failure_count is not None:
            return f"{source_prefix}Parallel fan-out completed ({success_count} success, {failure_count} failed)"
        return f"{source_prefix}Parallel fan-out completed"
    if raw_type == "InterviewStarted":
        return f"{source_prefix}Interview started for {node_id or 'human gate'}"
    if raw_type == "InterviewInform":
        message = (
            _as_trimmed_string(payload.get("message"))
            or _as_trimmed_string(payload.get("prompt"))
            or _as_trimmed_string(payload.get("question"))
        )
        return (
            f"{source_prefix}Interview info for {node_id or 'human gate'}: {message}"
            if message
            else f"{source_prefix}Interview info for {node_id or 'human gate'}"
        )
    if raw_type == "InterviewCompleted":
        answer = _as_trimmed_string(payload.get("answer"))
        provenance = _interview_outcome_provenance(raw_type, payload)
        if provenance == "skipped":
            return f"{source_prefix}Interview completed for {node_id or 'human gate'} (skipped)"
        if provenance == "accepted":
            return (
                f"{source_prefix}Interview completed for {node_id or 'human gate'} (accepted answer: {answer})"
                if answer
                else f"{source_prefix}Interview completed for {node_id or 'human gate'} (accepted answer)"
            )
        return (
            f"{source_prefix}Interview completed for {node_id or 'human gate'} ({answer})"
            if answer
            else f"{source_prefix}Interview completed for {node_id or 'human gate'}"
        )
    if raw_type == "InterviewTimeout":
        provenance = _interview_outcome_provenance(raw_type, payload)
        if provenance == "timeout_default_applied":
            default_choice = _interview_default_choice_label(payload)
            return (
                f"{source_prefix}Interview timed out for {node_id or 'human gate'} (default applied: {default_choice})"
                if default_choice
                else f"{source_prefix}Interview timed out for {node_id or 'human gate'} (default applied)"
            )
        if provenance == "timeout_no_default":
            return f"{source_prefix}Interview timed out for {node_id or 'human gate'} (no default applied)"
        return f"{source_prefix}Interview timed out for {node_id or 'human gate'}"
    if raw_type == "human_gate":
        prompt = _as_trimmed_string(payload.get("prompt"))
        return (
            f"{source_prefix}Human gate pending: {prompt}"
            if prompt
            else f"{source_prefix}Human gate pending for {node_id or 'unknown'}"
        )
    if raw_type == "CheckpointSaved":
        return f"{source_prefix}Checkpoint saved at {node_id or 'current node'}"
    if raw_type == "LLMContent":
        channel = _as_trimmed_string(payload.get("channel")) or "assistant"
        status = _as_trimmed_string(payload.get("status")) or "streaming"
        return f"{source_prefix}{channel.capitalize()} output {status} for {node_id or 'current node'}"
    return f"{source_prefix}{raw_type or 'event'}"


def _run_journal_entry_from_event(payload: dict[str, Any]) -> Optional[dict[str, Any]]:
    sequence = payload.get("sequence")
    emitted_at = payload.get("emitted_at")
    raw_type = _as_trimmed_string(payload.get("type"))
    if not isinstance(sequence, int) or not isinstance(emitted_at, str) or raw_type is None:
        return None

    node_id = _journal_node_id(payload)
    stage_index = _journal_stage_index(payload)
    source_scope = _journal_source_scope(payload)
    source_parent_node_id = _journal_source_parent_node_id(payload)
    source_flow_name = _journal_source_flow_name(payload)
    question_id = _journal_question_id(payload)
    return {
        "id": f"journal-{sequence}",
        "sequence": sequence,
        "emitted_at": emitted_at,
        "kind": _journal_kind(raw_type),
        "raw_type": raw_type,
        "severity": _journal_severity(raw_type, payload),
        "summary": _journal_summary(
            raw_type=raw_type,
            payload=payload,
            node_id=node_id,
            source_scope=source_scope,
            source_parent_node_id=source_parent_node_id,
            source_flow_name=source_flow_name,
        ),
        "node_id": node_id,
        "stage_index": stage_index,
        "source_scope": source_scope,
        "source_parent_node_id": source_parent_node_id,
        "source_flow_name": source_flow_name,
        "question_id": question_id,
        "payload": dict(payload),
    }


def _run_journal_entries(run_id: str) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for event in _read_persisted_run_events(run_id):
        entry = _run_journal_entry_from_event(event)
        if entry is not None:
            entries.append(entry)
    entries.sort(key=lambda item: item["sequence"], reverse=True)
    return entries


def _write_run_meta(record: RunRecord) -> None:
    pipeline_runs.write_run_meta(get_runtime_paths, record)


def _read_run_meta(path: Path) -> Optional[RunRecord]:
    return pipeline_runs.read_run_meta(path)


def _record_run_start(
    run_id: str,
    flow_name: str,
    working_directory: str,
    model: str,
    llm_provider: str = "codex",
    reasoning_effort: Optional[str] = None,
    spec_id: Optional[str] = None,
    plan_id: Optional[str] = None,
    continued_from_run_id: Optional[str] = None,
    continued_from_node: Optional[str] = None,
    continued_from_flow_mode: Optional[str] = None,
    continued_from_flow_name: Optional[str] = None,
    parent_run_id: Optional[str] = None,
    parent_node_id: Optional[str] = None,
    root_run_id: Optional[str] = None,
    child_invocation_index: Optional[int] = None,
) -> None:
    pipeline_runs.record_run_start(
        get_runtime_paths,
        RUN_HISTORY_LOCK,
        run_id=run_id,
        flow_name=flow_name,
        working_directory=working_directory,
        model=model,
        llm_provider=llm_provider,
        reasoning_effort=reasoning_effort,
        resolve_runtime_workspace_path=resolve_runtime_workspace_path,
        spec_id=spec_id,
        plan_id=plan_id,
        continued_from_run_id=continued_from_run_id,
        continued_from_node=continued_from_node,
        continued_from_flow_mode=continued_from_flow_mode,
        continued_from_flow_name=continued_from_flow_name,
        parent_run_id=parent_run_id,
        parent_node_id=parent_node_id,
        root_run_id=root_run_id,
        child_invocation_index=child_invocation_index,
    )


def _ensure_known_pipeline(pipeline_id: str) -> None:
    pipeline_runs.ensure_known_pipeline(get_runtime_paths, _get_active_run(pipeline_id), pipeline_id)


def _artifact_media_type(path: Path) -> str:
    return pipeline_runs.artifact_media_type(path)


def _artifact_is_viewable(*, media_type: str, path: Path) -> bool:
    return pipeline_runs.artifact_is_viewable(media_type=media_type, path=path)


def _resolve_artifact_path(run_root: Path, artifact_path: str) -> Path:
    return pipeline_runs.resolve_artifact_path(run_root, artifact_path)


def _list_run_output_artifacts(run_root: Path) -> List[Dict[str, object]]:
    return pipeline_runs.list_run_output_artifacts(run_root)


def _record_run_end(
    run_id: str,
    working_directory: str,
    status: str,
    last_error: str = "",
    *,
    outcome: str | None = None,
    outcome_reason_code: str | None = None,
    outcome_reason_message: str | None = None,
) -> None:
    pipeline_runs.record_run_end(
        get_runtime_paths,
        RUN_HISTORY_LOCK,
        run_id=run_id,
        working_directory=working_directory,
        status=status,
        last_error=last_error,
        outcome=outcome,
        outcome_reason_code=outcome_reason_code,
        outcome_reason_message=outcome_reason_message,
    )


def _record_run_status(
    run_id: str,
    status: str,
    last_error: str = "",
    *,
    outcome: str | None = None,
    outcome_reason_code: str | None = None,
    outcome_reason_message: str | None = None,
) -> None:
    pipeline_runs.record_run_status(
        get_runtime_paths,
        RUN_HISTORY_LOCK,
        run_id=run_id,
        status=status,
        last_error=last_error,
        outcome=outcome,
        outcome_reason_code=outcome_reason_code,
        outcome_reason_message=outcome_reason_message,
    )


def _record_run_usage(
    run_id: str,
    token_usage_breakdown: TokenUsageBreakdown,
) -> None:
    pipeline_runs.record_run_usage(
        get_runtime_paths,
        RUN_HISTORY_LOCK,
        run_id=run_id,
        token_usage_breakdown=token_usage_breakdown,
    )


def _append_run_log(run_id: str, message: str) -> None:
    pipeline_runs.append_run_log(get_runtime_paths, run_id, message)


async def _publish_run_event(run_id: str, message: dict) -> None:
    payload = dict(message)
    payload.setdefault("run_id", run_id)
    payload.setdefault("source_scope", "root")
    payload["sequence"] = _next_run_event_sequence(run_id)
    payload["emitted_at"] = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    if payload.get("type") == "log":
        _append_run_log(run_id, str(payload.get("msg", "")))
    _persist_run_event(run_id, payload)
    await manager.broadcast(payload)
    await EVENT_HUB.publish(run_id, payload)


async def _publish_lifecycle_phase(run_id: str, phase: str) -> None:
    await _publish_run_event(run_id, {"type": "lifecycle", "phase": phase})


def _publish_run_event_sync(loop: asyncio.AbstractEventLoop, run_id: str, message: dict) -> None:
    future = asyncio.run_coroutine_threadsafe(_publish_run_event(run_id, message), loop)
    try:
        future.result(timeout=10)
    except Exception:  # noqa: BLE001
        LOGGER.exception("failed to publish run event for run %s", run_id)


def _publish_run_list_upsert_sync(loop: asyncio.AbstractEventLoop, run_id: str) -> None:
    future = asyncio.run_coroutine_threadsafe(_publish_run_list_upsert(run_id), loop)
    try:
        future.result(timeout=10)
    except Exception:  # noqa: BLE001
        LOGGER.exception("failed to publish run list upsert for run %s", run_id)


def _set_active_run_status(run_id: str, status: str, *, last_error: Optional[str] = None) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.status = status
        if last_error is not None:
            run.last_error = last_error
        if status not in {"completed"}:
            run.outcome = None
            run.outcome_reason_code = None
            run.outcome_reason_message = None


def _terminal_status_summary(
    *,
    status: str,
    outcome: str | None,
    outcome_reason_code: str | None,
    outcome_reason_message: str | None,
    last_error: str = "",
) -> str:
    reason = (
        str(outcome_reason_message or "").strip()
        or str(last_error or "").strip()
        or str(outcome_reason_code or "").strip()
    )
    if status == "completed" and outcome:
        summary = f"Pipeline completed ({outcome})"
        return f"{summary}: {reason}" if reason else summary
    if status == "validation_error":
        summary = "Pipeline validation error"
        return f"{summary}: {reason}" if reason else summary
    summary = f"Pipeline {status}"
    return f"{summary}: {reason}" if reason else summary


def _set_active_run_outcome(
    run_id: str,
    *,
    outcome: str | None,
    outcome_reason_code: str | None,
    outcome_reason_message: str | None,
) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.outcome = outcome
        run.outcome_reason_code = outcome_reason_code
        run.outcome_reason_message = outcome_reason_message


def _set_active_run_completed_nodes(run_id: str, completed_nodes: List[str]) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.completed_nodes = list(completed_nodes)


def _set_active_run_usage(run_id: str, token_usage_breakdown: TokenUsageBreakdown) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.token_usage_breakdown = token_usage_breakdown.copy()
        run.token_usage = token_usage_breakdown.total_tokens
        run.estimated_model_cost = estimate_model_cost(token_usage_breakdown)


def _get_active_run(run_id: str) -> Optional[ActiveRun]:
    with ACTIVE_RUNS_LOCK:
        return ACTIVE_RUNS.get(run_id)


def _read_checkpoint_progress(run_id: str) -> tuple[str, List[str]]:
    return pipeline_runs.read_checkpoint_progress(get_runtime_paths, run_id)


def _pipeline_progress_payload(current_node: str, completed_nodes: List[str]) -> Dict[str, object]:
    return pipeline_runs.pipeline_progress_payload(current_node, completed_nodes)


def _read_hydrated_run_record(run_id: str) -> Optional[RunRecord]:
    record = _read_run_meta(_run_meta_path(run_id))
    if not record:
        return None
    _hydrate_run_record_launch_options(record, run_id)
    hydrate_run_record_from_log(record, _run_root(run_id))
    return record


def _hydrate_run_record_launch_options(record: RunRecord, run_id: str) -> None:
    checkpoint = load_checkpoint(_run_root(run_id) / "state.json")
    checkpoint_context = checkpoint.context if checkpoint is not None else {}
    if not str(record.llm_provider or "").strip():
        provider = str(checkpoint_context.get(RUNTIME_LAUNCH_PROVIDER_KEY, "")).strip().lower()
        record.llm_provider = provider or "codex"
    if record.reasoning_effort is None:
        reasoning_effort = str(checkpoint_context.get(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, "")).strip().lower()
        record.reasoning_effort = reasoning_effort or None


def _apply_active_run_to_record(record: RunRecord, active: Optional[ActiveRun]) -> RunRecord:
    if active is None:
        return record
    active_status = normalize_run_status(active.status)
    record.status = active_status
    record.outcome = active.outcome
    record.outcome_reason_code = active.outcome_reason_code
    record.outcome_reason_message = active.outcome_reason_message
    record.last_error = active.last_error
    record.llm_provider = active.llm_provider or record.llm_provider or "codex"
    record.reasoning_effort = active.reasoning_effort
    if active_status not in TERMINAL_RUN_STATUSES:
        record.ended_at = None
    if active.token_usage is not None:
        record.token_usage = active.token_usage
    if active.token_usage_breakdown is not None:
        record.token_usage_breakdown = active.token_usage_breakdown
        record.estimated_model_cost = active.estimated_model_cost
    return record


def _pipeline_status_payload(run_id: str) -> Dict[str, object]:
    checkpoint_current_node, checkpoint_completed_nodes = _read_checkpoint_progress(run_id)
    active = _get_active_run(run_id)
    record = _read_hydrated_run_record(run_id)

    if active is None and record is None:
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    if record is not None:
        payload = record.to_dict()
    else:
        payload = {
            "run_id": run_id,
            "flow_name": active.flow_name if active else "",
            "status": active.status if active else "unknown",
            "outcome": active.outcome if active else None,
            "outcome_reason_code": active.outcome_reason_code if active else None,
            "outcome_reason_message": active.outcome_reason_message if active else None,
            "working_directory": active.working_directory if active else "",
            "project_path": "",
            "git_branch": None,
            "git_commit": None,
            "spec_id": None,
            "plan_id": None,
            "model": active.model if active else "",
            "provider": active.llm_provider if active else "codex",
            "llm_provider": active.llm_provider if active else "codex",
            "reasoning_effort": active.reasoning_effort if active else None,
            "started_at": "",
            "ended_at": None,
            "last_error": active.last_error if active else "",
            "token_usage": None,
            "token_usage_breakdown": None,
            "estimated_model_cost": None,
            "continued_from_run_id": None,
            "continued_from_node": None,
            "continued_from_flow_mode": None,
            "continued_from_flow_name": None,
            "parent_run_id": None,
            "parent_node_id": None,
            "root_run_id": None,
            "child_invocation_index": None,
        }

    completed_nodes = list(active.completed_nodes) if active and active.completed_nodes else checkpoint_completed_nodes
    if active is not None:
        payload["status"] = active.status
        payload["outcome"] = active.outcome
        payload["outcome_reason_code"] = active.outcome_reason_code
        payload["outcome_reason_message"] = active.outcome_reason_message
        payload["last_error"] = active.last_error
        if active.token_usage is not None:
            payload["token_usage"] = active.token_usage
        if active.token_usage_breakdown is not None:
            payload["token_usage_breakdown"] = active.token_usage_breakdown.to_dict()
        if active.estimated_model_cost is not None:
            payload["estimated_model_cost"] = active.estimated_model_cost.to_dict()

    payload["pipeline_id"] = run_id
    payload["run_id"] = str(payload.get("run_id") or run_id)
    payload["completed_nodes"] = completed_nodes
    payload["progress"] = _pipeline_progress_payload(checkpoint_current_node, completed_nodes)
    return payload


def _list_run_records(project_path: Optional[str] = None) -> List[RunRecord]:
    records: List[RunRecord] = []
    for run_dir in _iter_run_roots():
        if not run_dir.is_dir():
            continue
        meta_path = run_dir / "run.json"
        record = _read_run_meta(meta_path)
        if record:
            _hydrate_run_record_launch_options(record, record.run_id)
            hydrate_run_record_from_log(record, run_dir)
            _apply_active_run_to_record(record, _get_active_run(record.run_id))
            records.append(record)
            continue

        run_id = run_dir.name
        record = RunRecord(
            run_id=run_id,
            flow_name="",
            status="unknown",
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory="",
            model="",
            llm_provider="codex",
            reasoning_effort=None,
            started_at="",
        )
        _hydrate_run_record_launch_options(record, run_id)
        hydrate_run_record_from_log(record, run_dir)
        _apply_active_run_to_record(record, _get_active_run(record.run_id))
        records.append(record)

    def _sort_key(item: RunRecord) -> str:
        return item.started_at or item.ended_at or ""

    if project_path:
        records = [record for record in records if run_matches_project_scope(record, project_path)]

    records.sort(key=_sort_key, reverse=True)
    return records


async def _publish_run_list_upsert(run_id: str) -> None:
    record = _read_hydrated_run_record(run_id)
    if record is None:
        return
    _apply_active_run_to_record(record, _get_active_run(run_id))
    await RUNS_EVENT_HUB.publish(
        {
            "type": "run_upsert",
            "run": record.to_dict(),
        }
    )


def _pop_active_run(run_id: str) -> Optional[ActiveRun]:
    with ACTIVE_RUNS_LOCK:
        return ACTIVE_RUNS.pop(run_id, None)


class PipelineStartRequest(BaseModel):
    run_id: Optional[str] = None
    flow_content: Optional[str] = None
    working_directory: str = "./workspace"
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    reasoning_effort: Optional[str] = None
    flow_name: Optional[str] = None
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    spec_id: Optional[str] = None
    plan_id: Optional[str] = None


class PipelineContinueRequest(BaseModel):
    start_node: str
    flow_source_mode: str
    flow_name: Optional[str] = None
    working_directory: Optional[str] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    reasoning_effort: Optional[str] = None


class PreviewRequest(BaseModel):
    flow_content: str
    flow_name: Optional[str] = None
    expand_children: bool = False


class SaveFlowRequest(BaseModel):
    name: str
    content: str
    expect_semantic_equivalence: bool = False


class ResetRequest(BaseModel):
    working_directory: str = "./workspace"


class HumanAnswerRequest(BaseModel):
    selected_value: str


class PipelineMetadataUpdateRequest(BaseModel):
    spec_id: Optional[str] = None
    plan_id: Optional[str] = None


DEFAULT_FLOW = """digraph SoftwareFactory {
    start [shape=Mdiamond, label="Start"];
    setup [shape=box, prompt="Initialize project"];
    build [shape=box, prompt="Build app"];
    done [shape=Msquare, label="Done"];

    start -> setup -> build -> done;
}"""

def _build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
    on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
) -> CodergenBackend:
    return _build_codergen_backend_impl(
        backend_name,
        working_dir,
        emit,
        model=model,
        on_usage_update=on_usage_update,
    )


@attractor_router.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    await manager.connect(websocket)
    try:
        while True:
            await websocket.receive_text()
    except WebSocketDisconnect:
        manager.disconnect(websocket)


@attractor_router.get("/status")
async def get_status():
    return {
        "status": RUNTIME.status,
        "outcome": RUNTIME.outcome,
        "outcome_reason_code": RUNTIME.outcome_reason_code,
        "outcome_reason_message": RUNTIME.outcome_reason_message,
        "last_error": RUNTIME.last_error,
        "last_working_directory": RUNTIME.last_working_directory,
        "last_model": RUNTIME.last_model,
        "last_completed_nodes": RUNTIME.last_completed_nodes,
        "last_flow_name": RUNTIME.last_flow_name,
    }


@attractor_router.get("/runs")
async def list_runs(project_path: Optional[str] = None):
    records = _list_run_records(project_path)
    return {"runs": [record.to_dict() for record in records]}


@attractor_router.get("/runs/events")
async def runs_events(request: Request, project_path: Optional[str] = None):
    queue = RUNS_EVENT_HUB.subscribe()
    try:
        snapshot_payload = {
            "type": "snapshot",
            "runs": [record.to_dict() for record in _list_run_records(project_path)],
        }
    except Exception:
        RUNS_EVENT_HUB.unsubscribe(queue)
        raise

    async def stream():
        try:
            yield f"data: {json.dumps(snapshot_payload)}\n\n"
            while True:
                if await request.is_disconnected():
                    break
                try:
                    event = await asyncio.wait_for(queue.get(), timeout=15.0)
                    if event.get("type") == "run_upsert" and project_path:
                        run_payload = event.get("run")
                        record = RunRecord.from_dict(run_payload) if isinstance(run_payload, dict) else None
                        if record is None or not run_matches_project_scope(record, project_path):
                            continue
                    yield f"data: {json.dumps(event)}\n\n"
                except asyncio.TimeoutError:
                    yield ": keepalive\n\n"
        finally:
            RUNS_EVENT_HUB.unsubscribe(queue)

    return StreamingResponse(
        stream(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    )


def _preview_attr_string(attr: object | None) -> str:
    if attr is None:
        return ""
    value = getattr(attr, "value", "")
    if hasattr(value, "raw"):
        value = value.raw
    return str(value).strip()


def _preview_authored_attr_string(attr: object | None) -> str:
    if attr is None or getattr(attr, "line", 0) == 0:
        return ""
    return _preview_attr_string(attr)


def _resolve_preview_path(raw_path: str | None) -> Path | None:
    if not raw_path:
        return None
    try:
        return Path(raw_path).expanduser().resolve()
    except OSError:
        return None


def _resolve_preview_path_from_base(raw_path: str | Path, base_dir: Path) -> Path:
    candidate = Path(raw_path)
    if candidate.is_absolute():
        return candidate.resolve()
    return (base_dir / candidate).resolve()


def _resolve_preview_child_workdir(
    *,
    authored_child_workdir: str,
    preview_base_dir: Path | None,
    run_workdir: Path | None,
) -> Path:
    base_dir = run_workdir or preview_base_dir or Path.cwd().resolve()
    if authored_child_workdir:
        return _resolve_preview_path_from_base(authored_child_workdir, base_dir)
    return run_workdir or preview_base_dir or Path.cwd().resolve()


def _resolve_preview_child_dot_path(
    child_dotfile: str,
    *,
    child_workdir_path: Path,
    flow_source_dir: Path | None,
    run_workdir: Path | None,
    child_workdir_is_authored: bool,
) -> Path:
    child_dot_path = Path(child_dotfile)
    if child_dot_path.is_absolute():
        return child_dot_path.resolve()
    if child_workdir_is_authored:
        base_dir = child_workdir_path
    else:
        base_dir = flow_source_dir or run_workdir or child_workdir_path
    return _resolve_preview_path_from_base(child_dot_path, base_dir)


def _is_manager_loop_node(node) -> bool:
    node_type = str(_dot_attr_value(node.attrs, "type", "") or "").strip()
    node_shape = str(_dot_attr_value(node.attrs, "shape", "") or "").strip()
    return node_type == "stack.manager_loop" or node_shape == "house"


def _build_child_preview_payload(
    graph,
    *,
    flow_source_dir: Path | None,
    run_workdir: Path | None,
) -> dict[str, dict]:
    child_dotfile = _preview_attr_string(graph.graph_attrs.get("stack.child_dotfile"))
    if not child_dotfile:
        return {}

    authored_child_workdir = _preview_authored_attr_string(graph.graph_attrs.get("stack.child_workdir"))
    child_workdir_path = _resolve_preview_child_workdir(
        authored_child_workdir=authored_child_workdir,
        preview_base_dir=flow_source_dir,
        run_workdir=run_workdir,
    )
    child_dot_path = _resolve_preview_child_dot_path(
        child_dotfile,
        child_workdir_path=child_workdir_path,
        flow_source_dir=flow_source_dir,
        run_workdir=run_workdir,
        child_workdir_is_authored=bool(authored_child_workdir),
    )
    if not child_dot_path.exists():
        return {}

    try:
        child_graph = parse_dot(child_dot_path.read_text(encoding="utf-8"))
    except (DotParseError, OSError):
        return {}

    child_graph, child_diagnostics = _prepare_graph_for_server(child_graph)
    if any(diag.severity == DiagnosticSeverity.ERROR for diag in child_diagnostics):
        return {}

    child_graph_payload = _graph_payload(child_graph)
    child_graph_attrs = child_graph_payload.get("graph_attrs", {})
    child_flow_label = ""
    if isinstance(child_graph_attrs, dict):
        raw_label = child_graph_attrs.get("label")
        if isinstance(raw_label, str):
            child_flow_label = raw_label

    return {
        node.node_id: {
            "flow_name": child_dot_path.name,
            "flow_path": str(child_dot_path),
            "flow_label": child_flow_label or child_dot_path.stem,
            "parent_node_id": node.node_id,
            "read_only": True,
            "provenance": "derived_child_preview",
            "graph": child_graph_payload,
        }
        for node in graph.nodes.values()
        if _is_manager_loop_node(node)
    }


def _graph_payload(graph, *, child_previews: dict[str, dict] | None = None) -> dict:
    canonical_graph = copy.deepcopy(graph)
    canonical_graph_attrs = canonical_graph.graph_attrs

    def _all_attrs_payload(attrs: Dict[str, object]) -> Dict[str, object]:
        return {key: _dot_attr_value(attrs, key) for key in attrs}

    def _merge_extension_attrs(payload: Dict[str, object], attrs: Dict[str, object]) -> Dict[str, object]:
        merged = dict(payload)
        for key, value in _all_attrs_payload(attrs).items():
            if key not in merged:
                merged[key] = value
        return merged

    def _defaults_payload(scope) -> dict:
        return {
            "node": _all_attrs_payload(scope.node),
            "edge": _all_attrs_payload(scope.edge),
        }

    def _subgraph_payload(scope) -> dict:
        return {
            "id": scope.id,
            "attrs": _all_attrs_payload(scope.attrs),
            "node_ids": list(scope.node_ids),
            "defaults": _defaults_payload(scope.defaults),
            "subgraphs": [_subgraph_payload(child) for child in scope.subgraphs],
        }

    payload = {
        "nodes": [
            _merge_extension_attrs({
                "id": n.node_id,
                "label": _dot_attr_value(n.attrs, "label", n.node_id),
                "shape": _dot_attr_value(n.attrs, "shape"),
                "prompt": _dot_attr_value(n.attrs, "prompt"),
                "tool.command": _dot_attr_value(n.attrs, "tool.command"),
                "tool.hooks.pre": _dot_attr_value(n.attrs, "tool.hooks.pre"),
                "tool.hooks.post": _dot_attr_value(n.attrs, "tool.hooks.post"),
                "tool.artifacts.paths": _dot_attr_value(n.attrs, "tool.artifacts.paths"),
                "tool.artifacts.stdout": _dot_attr_value(n.attrs, "tool.artifacts.stdout"),
                "tool.artifacts.stderr": _dot_attr_value(n.attrs, "tool.artifacts.stderr"),
                "join_policy": _dot_attr_value(n.attrs, "join_policy"),
                "error_policy": _dot_attr_value(n.attrs, "error_policy"),
                "max_parallel": _dot_attr_value(n.attrs, "max_parallel"),
                "type": _dot_attr_value(n.attrs, "type"),
                "max_retries": _dot_attr_value(n.attrs, "max_retries"),
                "goal_gate": _dot_attr_value(n.attrs, "goal_gate"),
                "retry_target": _dot_attr_value(n.attrs, "retry_target"),
                "fallback_retry_target": _dot_attr_value(n.attrs, "fallback_retry_target"),
                "fidelity": _dot_attr_value(n.attrs, "fidelity"),
                "thread_id": _dot_attr_value(n.attrs, "thread_id"),
                "class": _dot_attr_value(n.attrs, "class"),
                "timeout": _dot_attr_value(n.attrs, "timeout"),
                "llm_model": _dot_attr_value(n.attrs, "llm_model"),
                "llm_provider": _dot_attr_value(n.attrs, "llm_provider"),
                "reasoning_effort": _dot_attr_value(n.attrs, "reasoning_effort"),
                "auto_status": _dot_attr_value(n.attrs, "auto_status"),
                "allow_partial": _dot_attr_value(n.attrs, "allow_partial"),
                "manager.poll_interval": _dot_attr_value(n.attrs, "manager.poll_interval"),
                "manager.max_cycles": _dot_attr_value(n.attrs, "manager.max_cycles"),
                "manager.stop_condition": _dot_attr_value(n.attrs, "manager.stop_condition"),
                "manager.actions": _dot_attr_value(n.attrs, "manager.actions"),
                "human.default_choice": _dot_attr_value(n.attrs, "human.default_choice"),
            }, n.attrs)
            for n in graph.nodes.values()
        ],
        "graph_attrs": _merge_extension_attrs({
            "goal": _dot_attr_value(canonical_graph_attrs, "goal"),
            "label": _dot_attr_value(canonical_graph_attrs, "label", ""),
            "model_stylesheet": _dot_attr_value(canonical_graph_attrs, "model_stylesheet"),
            DEFAULT_MAX_RETRIES_KEY: _dot_attr_value(canonical_graph_attrs, DEFAULT_MAX_RETRIES_KEY),
            "retry_target": _dot_attr_value(canonical_graph_attrs, "retry_target"),
            "fallback_retry_target": _dot_attr_value(canonical_graph_attrs, "fallback_retry_target"),
            "default_fidelity": _dot_attr_value(canonical_graph_attrs, "default_fidelity"),
            "stack.child_dotfile": _dot_attr_value(canonical_graph_attrs, "stack.child_dotfile"),
            "stack.child_workdir": _dot_attr_value(canonical_graph_attrs, "stack.child_workdir"),
            "tool.hooks.pre": _dot_attr_value(canonical_graph_attrs, "tool.hooks.pre"),
            "tool.hooks.post": _dot_attr_value(canonical_graph_attrs, "tool.hooks.post"),
            "ui_default_llm_model": _dot_attr_value(canonical_graph_attrs, "ui_default_llm_model"),
            "ui_default_llm_provider": _dot_attr_value(canonical_graph_attrs, "ui_default_llm_provider"),
            "ui_default_reasoning_effort": _dot_attr_value(canonical_graph_attrs, "ui_default_reasoning_effort"),
        }, canonical_graph_attrs),
        "edges": [
            _merge_extension_attrs({
                "from": e.source,
                "to": e.target,
                "label": _dot_attr_value(e.attrs, "label"),
                "condition": _dot_attr_value(e.attrs, "condition"),
                "weight": _dot_attr_value(e.attrs, "weight"),
                "fidelity": _dot_attr_value(e.attrs, "fidelity"),
                "thread_id": _dot_attr_value(e.attrs, "thread_id"),
                "loop_restart": _dot_attr_value(e.attrs, "loop_restart"),
            }, e.attrs)
            for e in graph.edges
        ],
        "defaults": _defaults_payload(graph.defaults),
        "subgraphs": [_subgraph_payload(subgraph) for subgraph in graph.subgraphs],
    }
    if child_previews:
        payload["child_previews"] = child_previews
    return payload


def _diagnostic_payload(diagnostic: Diagnostic) -> dict:
    return _diagnostic_payload_impl(diagnostic)


def _build_transform_pipeline() -> object:
    return _build_graph_transform_pipeline(_registered_transforms_snapshot())


def _prepare_graph_for_server(graph):
    return _prepare_graph_impl(
        graph,
        extra_transforms=_registered_transforms_snapshot(),
    )


def _preview_payload_from_dot_source(
    dot_source: str,
    *,
    expand_children: bool = False,
    flow_source_dir: Path | None = None,
    run_workdir: Path | None = None,
) -> dict:
    try:
        graph = parse_dot(dot_source)
    except DotParseError as exc:
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
            "node_id": None,
        }
        return {
            "status": "parse_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    graph, diagnostics = _prepare_graph_for_server(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    child_previews = _build_child_preview_payload(
        graph,
        flow_source_dir=flow_source_dir,
        run_workdir=run_workdir,
    ) if expand_children else None
    return {
        "status": "ok" if not errors else "validation_error",
        "graph": _graph_payload(graph, child_previews=child_previews),
        "diagnostics": [_diagnostic_payload(d) for d in diagnostics],
        "errors": [_diagnostic_payload(d) for d in errors],
    }


@attractor_router.post("/preview")
async def preview_pipeline(req: PreviewRequest):
    flow_source_dir: Path | None = None
    flow_name = (req.flow_name or "").strip()
    if flow_name:
        try:
            flow_path = _resolve_flow_path(flow_name)
        except HTTPException:
            flow_path = None
        if flow_path is not None:
            flow_source_dir = flow_path.parent.resolve()

    return _preview_payload_from_dot_source(
        req.flow_content,
        expand_children=req.expand_children,
        flow_source_dir=flow_source_dir,
    )


def _resolve_run_preview_source_context(pipeline_id: str) -> tuple[Path | None, Path | None]:
    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    context = checkpoint.context if checkpoint is not None else {}
    flow_source_dir = _resolve_preview_path(str(context.get("internal.flow_source_dir", "")).strip())
    run_workdir = _resolve_preview_path(str(context.get("internal.run_workdir", "")).strip())
    if run_workdir is not None:
        return flow_source_dir, run_workdir

    record = _read_run_meta(_run_meta_path(pipeline_id))
    if record and record.working_directory:
        return flow_source_dir, _resolve_preview_path(record.working_directory)

    active = _get_active_run(pipeline_id)
    if active and active.working_directory:
        return flow_source_dir, _resolve_preview_path(active.working_directory)

    return flow_source_dir, None


def _resolve_run_graph_source_path(pipeline_id: str) -> Path:
    graph_dir = _run_root(pipeline_id) / "artifacts" / "graphviz"
    source_path = graph_dir / "pipeline-source.dot"
    fallback_path = graph_dir / "pipeline.dot"
    graph_source_path = source_path if source_path.exists() else fallback_path
    if not graph_source_path.exists():
        raise HTTPException(status_code=404, detail="Run graph preview unavailable")
    return graph_source_path


def _resolve_continue_source_record(pipeline_id: str) -> tuple[RunRecord, Checkpoint]:
    active = _get_active_run(pipeline_id)
    record = _read_run_meta(_run_meta_path(pipeline_id))
    if not active and record is None:
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=404, detail="Checkpoint unavailable")
    if record is None:
        assert active is not None
        record = RunRecord(
            run_id=pipeline_id,
            flow_name=active.flow_name,
            status=active.status,
            outcome=active.outcome,
            outcome_reason_code=active.outcome_reason_code,
            outcome_reason_message=active.outcome_reason_message,
            working_directory=active.working_directory,
            model=active.model,
            llm_provider=active.llm_provider,
            reasoning_effort=active.reasoning_effort,
            started_at="",
        )
    else:
        _hydrate_run_record_launch_options(record, pipeline_id)
    return record, checkpoint


def _resolve_continue_flow_source(
    *,
    pipeline_id: str,
    source_record: RunRecord,
    flow_source_mode: str,
    flow_name: Optional[str],
) -> tuple[str, str, Path | None]:
    normalized_mode = flow_source_mode.strip().lower()
    if normalized_mode not in {"snapshot", "flow_name"}:
        raise ValueError("flow_source_mode must be either snapshot or flow_name.")

    if normalized_mode == "snapshot":
        graph_source_path = _resolve_run_graph_source_path(pipeline_id)
        return (
            source_record.flow_name,
            graph_source_path.read_text(encoding="utf-8"),
            None,
        )

    selected_flow_name = (flow_name or "").strip()
    if not selected_flow_name:
        raise ValueError("flow_name is required when flow_source_mode is flow_name.")
    flow_content = _load_flow_content(selected_flow_name)
    flow_path = _resolve_flow_path(selected_flow_name)
    return (
        selected_flow_name,
        flow_content,
        flow_path.parent.resolve(),
    )


def _run_executor_in_workdir(executor: PipelineExecutor, context: Context, workdir: Path, *, resume: bool) -> object:
    original_cwd = Path.cwd()
    os.chdir(workdir)
    try:
        return executor.run(context, resume=resume)
    finally:
        os.chdir(original_cwd)


def _clear_child_runtime_snapshot(context: Context) -> None:
    context.apply_updates(
        {
            "context.stack.child.run_id": "",
            "context.stack.child.status": "",
            "context.stack.child.outcome": "",
            "context.stack.child.outcome_reason_code": "",
            "context.stack.child.outcome_reason_message": "",
            "context.stack.child.active_stage": "",
            "context.stack.child.completed_nodes": [],
            "context.stack.child.route_trace": [],
            "context.stack.child.failure_reason": "",
            "context.stack.child.retry_count": "",
            "context.stack.child.intervention": "",
        }
    )


def _child_run_result_from_pipeline_result(run_id: str, result) -> ChildRunResult:
    final_status = normalize_run_status(result.status)
    if final_status == "aborted":
        final_status = "canceled"
    return ChildRunResult(
        run_id=run_id,
        status=final_status,
        outcome=result.outcome,
        outcome_reason_code=result.outcome_reason_code,
        outcome_reason_message=result.outcome_reason_message,
        current_node=result.current_node,
        completed_nodes=list(result.completed_nodes),
        route_trace=list(result.route_trace),
        failure_reason=result.failure_reason or "",
    )


def _child_run_result_from_record(run_id: str) -> ChildRunResult | None:
    active = _get_active_run(run_id)
    record = _read_hydrated_run_record(run_id)
    if active is None and record is None:
        return None
    checkpoint = load_checkpoint(_run_root(run_id) / "state.json")
    current_node = checkpoint.current_node if checkpoint is not None else ""
    completed_nodes = list(checkpoint.completed_nodes) if checkpoint is not None else []
    checkpoint_context = checkpoint.context if checkpoint is not None else {}
    route_trace = checkpoint_context.get("context.stack.child.route_trace", [])
    if not isinstance(route_trace, list):
        route_trace = []

    if active is not None:
        return ChildRunResult(
            run_id=run_id,
            status=normalize_run_status(active.status),
            outcome=active.outcome,
            outcome_reason_code=active.outcome_reason_code,
            outcome_reason_message=active.outcome_reason_message,
            current_node=current_node,
            completed_nodes=list(active.completed_nodes or completed_nodes),
            route_trace=list(route_trace),
            failure_reason=active.last_error or "",
        )

    assert record is not None
    return ChildRunResult(
        run_id=run_id,
        status=normalize_run_status(record.status),
        outcome=record.outcome,
        outcome_reason_code=record.outcome_reason_code,
        outcome_reason_message=record.outcome_reason_message,
        current_node=current_node,
        completed_nodes=completed_nodes,
        route_trace=list(route_trace),
        failure_reason=record.last_error or "",
    )


def _next_child_invocation_index(parent_run_id: str, parent_node_id: str) -> int:
    max_index = 0
    for run_root in _iter_run_roots():
        record = _read_run_meta(run_root / "run.json")
        if record is None:
            continue
        if record.parent_run_id != parent_run_id or record.parent_node_id != parent_node_id:
            continue
        if record.child_invocation_index is not None:
            max_index = max(max_index, record.child_invocation_index)
    return max_index + 1


def _combined_execution_control(
    *controls: Callable[[], Optional[str]] | None,
) -> Callable[[], Optional[str]]:
    def poll() -> Optional[str]:
        deferred_action: str | None = None
        for control in controls:
            if control is None:
                continue
            try:
                action = control()
            except Exception:  # noqa: BLE001
                continue
            if action == "abort":
                return "abort"
            if action and deferred_action is None:
                deferred_action = action
        return deferred_action

    return poll


def _run_first_class_child_pipeline(
    request: ChildRunRequest,
    *,
    backend_name: str,
    model: Optional[str],
    loop: asyncio.AbstractEventLoop,
) -> ChildRunResult:
    child_run_id = request.child_run_id.strip() or uuid.uuid4().hex
    working_dir = str(request.child_workdir.expanduser().resolve())
    os.makedirs(working_dir, exist_ok=True)
    run_root = _ensure_run_root_for_project(child_run_id, working_dir)
    logs_root = str(run_root / "logs")
    checkpoint_file = str(run_root / "state.json")
    Path(logs_root).mkdir(parents=True, exist_ok=True)
    selected_model, display_model = _resolve_launch_model(request.child_graph, model)
    selected_provider = str(request.parent_context.get(RUNTIME_LAUNCH_PROVIDER_KEY, "")).strip().lower() or "codex"
    selected_reasoning_effort = (
        str(request.parent_context.get(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, "")).strip().lower() or None
    )
    root_run_id = (request.root_run_id or request.parent_run_id or child_run_id).strip() or child_run_id
    child_invocation_index = _next_child_invocation_index(request.parent_run_id, request.parent_node_id)

    try:
        flow_content = request.child_flow_path.read_text(encoding="utf-8")
    except OSError:
        flow_content = f"digraph {request.child_graph.graph_id or 'child'} {{}}"
    graphviz_export = export_graphviz_artifact(flow_content, run_root)

    parent_launch_context = dict(request.parent_context.values)
    try:
        _validate_graph_provider_configuration(
            request.child_graph,
            launch_context=parent_launch_context,
            selected_provider=selected_provider,
            selected_model=selected_model,
            selected_reasoning_effort=selected_reasoning_effort,
        )
    except ProviderConfigurationError as exc:
        return ChildRunResult(run_id=child_run_id, status="failed", failure_reason=str(exc))

    _record_run_start(
        child_run_id,
        request.child_flow_name,
        working_dir,
        display_model,
        llm_provider=selected_provider,
        reasoning_effort=selected_reasoning_effort,
        parent_run_id=request.parent_run_id,
        parent_node_id=request.parent_node_id,
        root_run_id=root_run_id,
        child_invocation_index=child_invocation_index,
    )
    control = ExecutionControl()
    with ACTIVE_RUNS_LOCK:
        ACTIVE_RUNS[child_run_id] = ActiveRun(
            run_id=child_run_id,
            flow_name=request.child_flow_name,
            working_directory=working_dir,
            model=display_model,
            llm_provider=selected_provider,
            reasoning_effort=selected_reasoning_effort,
            status="running",
            control=control,
        )
    _publish_run_list_upsert_sync(loop, child_run_id)

    def emit(message: dict):
        _publish_run_event_sync(loop, child_run_id, message)

    def handle_usage_update(token_usage_breakdown: TokenUsageBreakdown) -> None:
        _record_run_usage(child_run_id, token_usage_breakdown)
        _set_active_run_usage(child_run_id, token_usage_breakdown)
        _publish_run_list_upsert_sync(loop, child_run_id)

    try:
        backend = _build_codergen_backend(
            backend_name,
            working_dir,
            emit,
            model=selected_model,
            on_usage_update=handle_usage_update,
        )
    except ValueError as exc:
        _set_active_run_status(child_run_id, "failed", last_error=str(exc))
        _record_run_end(child_run_id, working_dir, "failed", str(exc), outcome=None)
        _publish_run_list_upsert_sync(loop, child_run_id)
        _pop_active_run(child_run_id)
        return ChildRunResult(run_id=child_run_id, status="failed", failure_reason=str(exc))
    interviewer: Interviewer = WebInterviewer(HUMAN_BROKER, emit, request.child_flow_name, child_run_id)
    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )

    def launch_nested_child(nested_request: ChildRunRequest) -> ChildRunResult:
        return _run_first_class_child_pipeline(
            nested_request,
            backend_name=backend_name,
            model=model,
            loop=loop,
        )

    runner = BroadcastingRunner(
        HandlerRunner(
            request.child_graph,
            registry,
            child_run_launcher=launch_nested_child,
            child_status_resolver=_child_run_result_from_record,
        ),
        emit,
    )
    context = request.parent_context.clone()
    _clear_child_runtime_snapshot(context)
    context.apply_updates(_graph_attr_context_seed(request.child_graph))
    start_node = _resolve_start_node_id(request.child_graph)
    context.set("current_node", start_node)
    context.set("outcome", "")
    context.set("preferred_label", "")
    context.set(RUNTIME_LAUNCH_MODEL_KEY, selected_model or "")
    context.set(RUNTIME_LAUNCH_PROVIDER_KEY, selected_provider)
    context.set(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, selected_reasoning_effort or "")
    context.set("internal.run_id", child_run_id)
    context.set("internal.parent_run_id", request.parent_run_id)
    context.set("internal.parent_node_id", request.parent_node_id)
    context.set("internal.root_run_id", root_run_id)
    context.set("internal.run_workdir", working_dir)
    context.set("internal.flow_source_dir", str(request.child_flow_path.parent.resolve()))

    save_checkpoint(
        Path(checkpoint_file),
        Checkpoint(
            current_node=start_node,
            completed_nodes=[],
            context=dict(context.values),
            retry_counts={},
        ),
    )

    executor = PipelineExecutor(
        request.child_graph,
        runner,
        logs_root=logs_root,
        checkpoint_file=checkpoint_file,
        control=_combined_execution_control(control.poll, request.control),
        on_event=emit,
    )
    emit(
        {
            "type": "graph",
            **_graph_payload(request.child_graph),
        }
    )
    emit(
        {
            "type": "run_meta",
            "working_directory": working_dir,
            "model": display_model,
            "provider": selected_provider,
            "llm_provider": selected_provider,
            "reasoning_effort": selected_reasoning_effort,
            "flow_name": request.child_flow_name,
            "run_id": child_run_id,
            "parent_run_id": request.parent_run_id,
            "parent_node_id": request.parent_node_id,
            "root_run_id": root_run_id,
            "child_invocation_index": child_invocation_index,
            "graph_source_path": str(graphviz_export.source_path),
            "graph_dot_path": str(graphviz_export.dot_path),
            "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
        }
    )
    emit(
        {
            "type": "log",
            "msg": f"[System] Launching child run {child_run_id} in {working_dir} with model: {display_model}",
        }
    )
    if graphviz_export.error:
        emit({"type": "log", "msg": f"[System] Graph render unavailable: {graphviz_export.error}"})

    final_status = "failed"
    child_result = ChildRunResult(run_id=child_run_id, status="failed")
    try:
        _publish_run_event_sync(loop, child_run_id, {"type": "lifecycle", "phase": PIPELINE_LIFECYCLE_PHASES[4]})
        result = _run_executor_in_workdir(executor, context, request.child_workdir, resume=False)
        child_result = _child_run_result_from_pipeline_result(child_run_id, result)
        final_status = child_result.status
        final_last_error = ""
        if final_status in {"failed", "validation_error"}:
            final_last_error = (
                str(child_result.failure_reason or "").strip()
                or str(child_result.outcome_reason_message or "").strip()
                or str(child_result.outcome_reason_code or "").strip()
            )
        _set_active_run_status(
            child_run_id,
            final_status,
            last_error=final_last_error if final_last_error else None,
        )
        _set_active_run_outcome(
            child_run_id,
            outcome=child_result.outcome,
            outcome_reason_code=child_result.outcome_reason_code,
            outcome_reason_message=child_result.outcome_reason_message,
        )
        _set_active_run_completed_nodes(child_run_id, child_result.completed_nodes)
        emit(
            {
                "type": "runtime",
                "status": final_status,
                "outcome": child_result.outcome,
                "outcome_reason_code": child_result.outcome_reason_code,
                "outcome_reason_message": child_result.outcome_reason_message,
                "last_error": final_last_error or None,
            }
        )
        _record_run_end(
            child_run_id,
            working_dir,
            final_status,
            final_last_error,
            outcome=child_result.outcome,
            outcome_reason_code=child_result.outcome_reason_code,
            outcome_reason_message=child_result.outcome_reason_message,
        )
        _publish_run_list_upsert_sync(loop, child_run_id)
        emit(
            {
                "type": "log",
                "msg": _terminal_status_summary(
                    status=final_status,
                    outcome=child_result.outcome,
                    outcome_reason_code=child_result.outcome_reason_code,
                    outcome_reason_message=child_result.outcome_reason_message,
                    last_error=final_last_error,
                ),
            }
        )
    except Exception as exc:  # noqa: BLE001
        final_status = "failed"
        child_result = ChildRunResult(run_id=child_run_id, status="failed", failure_reason=str(exc))
        _set_active_run_status(child_run_id, "failed", last_error=str(exc))
        _set_active_run_outcome(
            child_run_id,
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
        )
        emit(
            {
                "type": "runtime",
                "status": "failed",
                "outcome": None,
                "outcome_reason_code": None,
                "outcome_reason_message": None,
                "last_error": str(exc),
            }
        )
        _record_run_end(child_run_id, working_dir, "failed", str(exc), outcome=None)
        _publish_run_list_upsert_sync(loop, child_run_id)
        emit({"type": "log", "msg": f"Pipeline Failed: {exc}"})
    finally:
        _publish_run_event_sync(loop, child_run_id, {"type": "lifecycle", "phase": PIPELINE_LIFECYCLE_PHASES[5]})
        _pop_active_run(child_run_id)
        _publish_run_list_upsert_sync(loop, child_run_id)
    return child_result


def _record_run_retry_start(
    run_id: str,
    *,
    llm_provider: str,
    reasoning_effort: str | None,
) -> None:
    with RUN_HISTORY_LOCK:
        record = _read_run_meta(_run_meta_path(run_id))
        if record is None:
            return
        record.llm_provider = llm_provider or record.llm_provider or "codex"
        record.reasoning_effort = reasoning_effort
        record.status = "running"
        record.outcome = None
        record.outcome_reason_code = None
        record.outcome_reason_message = None
        record.ended_at = None
        record.last_error = ""
        _write_run_meta(record)


def _prepare_checkpoint_for_retry(run_id: str, checkpoint: Checkpoint) -> Checkpoint:
    current_node = checkpoint.current_node
    completed_nodes = list(checkpoint.completed_nodes)
    checkpoint_context = dict(checkpoint.context)
    checkpoint_context[PIPELINE_RETRY_RUN_ID_CONTEXT_KEY] = run_id
    node_outcomes = checkpoint.context.get("_attractor.node_outcomes", {})
    current_outcome = node_outcomes.get(current_node) if isinstance(node_outcomes, dict) else None
    if current_node and current_outcome == "fail" and current_node in completed_nodes:
        completed_nodes = [node_id for node_id in completed_nodes if node_id != current_node]
    prepared = Checkpoint(
        current_node=current_node,
        completed_nodes=completed_nodes,
        context=checkpoint_context,
        retry_counts=dict(checkpoint.retry_counts),
        logs=list(checkpoint.logs),
    )

    return prepared


def _save_retry_checkpoint(run_id: str, checkpoint: Checkpoint) -> None:
    save_checkpoint(_run_root(run_id) / "state.json", checkpoint)
    checkpoint_json_path = _run_root(run_id) / "checkpoint.json"
    if checkpoint_json_path.exists():
        save_checkpoint(checkpoint_json_path, checkpoint)


def _build_pipeline_runner_for_run(
    *,
    graph,
    run_id: str,
    flow_name: str,
    working_dir: str,
    backend_name: str,
    model: Optional[str],
    llm_provider: Optional[str],
    reasoning_effort: Optional[str],
    loop: asyncio.AbstractEventLoop,
    on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
) -> tuple[PipelineExecutor, Context, str, Optional[str], Optional[str]]:
    selected_model, display_model = _resolve_launch_model(graph, model)
    selected_provider = _resolve_launch_provider(graph, llm_provider)
    selected_reasoning_effort = _resolve_launch_reasoning_effort(graph, reasoning_effort)

    _validate_graph_provider_configuration(
        graph,
        launch_context=None,
        selected_provider=selected_provider,
        selected_model=selected_model,
        selected_reasoning_effort=selected_reasoning_effort,
    )

    def emit(message: dict):
        _publish_run_event_sync(loop, run_id, message)

    backend = _build_codergen_backend(
        backend_name,
        working_dir,
        emit,
        model=selected_model,
        on_usage_update=on_usage_update,
    )
    interviewer: Interviewer = WebInterviewer(HUMAN_BROKER, emit, flow_name, run_id)
    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )

    def launch_child_run(child_request: ChildRunRequest) -> ChildRunResult:
        return _run_first_class_child_pipeline(
            child_request,
            backend_name=backend_name,
            model=model,
            loop=loop,
        )

    runner = BroadcastingRunner(
        HandlerRunner(
            graph,
            registry,
            child_run_launcher=launch_child_run,
            child_status_resolver=_child_run_result_from_record,
        ),
        emit,
    )
    control = ExecutionControl()
    executor = PipelineExecutor(
        graph,
        runner,
        logs_root=str(_run_root(run_id) / "logs"),
        checkpoint_file=str(_run_root(run_id) / "state.json"),
        control=control.poll,
        on_event=emit,
    )
    context = Context()
    with ACTIVE_RUNS_LOCK:
        ACTIVE_RUNS[run_id] = ActiveRun(
            run_id=run_id,
            flow_name=flow_name,
            working_directory=working_dir,
            model=display_model,
            llm_provider=selected_provider,
            reasoning_effort=selected_reasoning_effort,
            status="running",
            control=control,
        )
    return executor, context, display_model, selected_provider, selected_reasoning_effort


async def _retry_pipeline_run(pipeline_id: str) -> dict:
    if _get_active_run(pipeline_id) is not None:
        raise HTTPException(status_code=409, detail="Pipeline is already running")
    record = _read_hydrated_run_record(pipeline_id)
    if record is None:
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    if normalize_run_status(record.status) != "failed":
        raise HTTPException(status_code=409, detail="Retry requires a failed pipeline")
    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=409, detail="Retry requires an available checkpoint")
    try:
        graph_source_path = _resolve_run_graph_source_path(pipeline_id)
    except HTTPException as exc:
        raise HTTPException(status_code=409, detail="Retry requires an available graph snapshot") from exc

    flow_content = graph_source_path.read_text(encoding="utf-8")
    try:
        graph = parse_dot(flow_content)
    except DotParseError as exc:
        raise HTTPException(status_code=409, detail=f"Stored graph snapshot is invalid: {exc}") from exc
    graph, diagnostics = _prepare_graph_for_server(graph)
    errors = [diag for diag in diagnostics if diag.severity == DiagnosticSeverity.ERROR]
    if errors:
        raise HTTPException(status_code=409, detail=f"Stored graph snapshot failed validation: {errors[0].message}")

    raw_working_dir = str(record.working_directory or record.project_path or "").strip()
    if not raw_working_dir:
        raise HTTPException(status_code=409, detail="Retry requires a working directory")
    working_dir = str(Path(raw_working_dir).expanduser().resolve())
    os.makedirs(working_dir, exist_ok=True)
    loop = asyncio.get_running_loop()

    def handle_usage_update(token_usage_breakdown: TokenUsageBreakdown) -> None:
        _record_run_usage(pipeline_id, token_usage_breakdown)
        _set_active_run_usage(pipeline_id, token_usage_breakdown)
        _publish_run_list_upsert_sync(loop, pipeline_id)

    try:
        retry_provider = (
            str(checkpoint.context.get(RUNTIME_LAUNCH_PROVIDER_KEY, "")).strip().lower()
            or record.llm_provider
            or "codex"
        )
        retry_reasoning_effort = (
            str(checkpoint.context.get(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, "")).strip().lower()
            or record.reasoning_effort
            or None
        )
        executor, context, display_model, selected_provider, selected_reasoning_effort = _build_pipeline_runner_for_run(
            graph=graph,
            run_id=pipeline_id,
            flow_name=record.flow_name,
            working_dir=working_dir,
            backend_name="provider-router",
            model=record.model,
            llm_provider=retry_provider,
            reasoning_effort=retry_reasoning_effort,
            loop=loop,
            on_usage_update=handle_usage_update,
        )
    except ValueError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc

    checkpoint = _prepare_checkpoint_for_retry(pipeline_id, checkpoint)
    _save_retry_checkpoint(pipeline_id, checkpoint)
    _record_run_retry_start(
        pipeline_id,
        llm_provider=selected_provider,
        reasoning_effort=selected_reasoning_effort,
    )
    await _publish_run_list_upsert(pipeline_id)
    await _publish_run_event(
        pipeline_id,
        {
            "type": "PipelineRetryStarted",
            "current_node": checkpoint.current_node,
            "completed_nodes": list(checkpoint.completed_nodes),
        },
    )
    await _publish_run_event(
        pipeline_id,
        {
            "type": "runtime",
            "status": "running",
            "outcome": None,
            "outcome_reason_code": None,
            "outcome_reason_message": None,
            "last_error": None,
        },
    )

    async def _run_retry():
        final_status = "failed"
        try:
            await _publish_lifecycle_phase(pipeline_id, PIPELINE_LIFECYCLE_PHASES[4])
            result = await asyncio.to_thread(
                _run_executor_in_workdir,
                executor,
                context,
                Path(working_dir),
                resume=True,
            )
            final_status = normalize_run_status(result.status)
            final_outcome = result.outcome
            final_outcome_reason_code = result.outcome_reason_code
            final_outcome_reason_message = result.outcome_reason_message
            final_last_error = ""
            if final_status in {"failed", "validation_error"}:
                final_last_error = (
                    str(result.failure_reason or "").strip()
                    or str(final_outcome_reason_message or "").strip()
                    or str(final_outcome_reason_code or "").strip()
                )
            _set_active_run_status(
                pipeline_id,
                final_status,
                last_error=final_last_error if final_last_error else None,
            )
            _set_active_run_outcome(
                pipeline_id,
                outcome=final_outcome,
                outcome_reason_code=final_outcome_reason_code,
                outcome_reason_message=final_outcome_reason_message,
            )
            _set_active_run_completed_nodes(pipeline_id, result.completed_nodes)
            await _publish_run_event(
                pipeline_id,
                {
                    "type": "runtime",
                    "status": final_status,
                    "outcome": final_outcome,
                    "outcome_reason_code": final_outcome_reason_code,
                    "outcome_reason_message": final_outcome_reason_message,
                    "last_error": final_last_error or None,
                },
            )
            _record_run_end(
                pipeline_id,
                working_dir,
                final_status,
                final_last_error,
                outcome=final_outcome,
                outcome_reason_code=final_outcome_reason_code,
                outcome_reason_message=final_outcome_reason_message,
            )
            await _publish_run_list_upsert(pipeline_id)
            await _publish_run_event(
                pipeline_id,
                {
                    "type": "PipelineRetryCompleted",
                    "status": final_status,
                    "outcome": final_outcome,
                    "outcome_reason_code": final_outcome_reason_code,
                    "outcome_reason_message": final_outcome_reason_message,
                    "last_error": final_last_error or None,
                },
            )
            await _publish_run_event(
                pipeline_id,
                {
                    "type": "log",
                    "msg": _terminal_status_summary(
                        status=final_status,
                        outcome=final_outcome,
                        outcome_reason_code=final_outcome_reason_code,
                        outcome_reason_message=final_outcome_reason_message,
                        last_error=final_last_error,
                    ),
                },
            )
        except Exception as exc:  # noqa: BLE001
            final_status = "failed"
            _set_active_run_status(pipeline_id, "failed", last_error=str(exc))
            _set_active_run_outcome(
                pipeline_id,
                outcome=None,
                outcome_reason_code=None,
                outcome_reason_message=None,
            )
            await _publish_run_event(
                pipeline_id,
                {
                    "type": "runtime",
                    "status": "failed",
                    "outcome": None,
                    "outcome_reason_code": None,
                    "outcome_reason_message": None,
                    "last_error": str(exc),
                },
            )
            _record_run_end(pipeline_id, working_dir, "failed", str(exc), outcome=None)
            await _publish_run_list_upsert(pipeline_id)
            await _publish_run_event(
                pipeline_id,
                {"type": "PipelineRetryCompleted", "status": "failed", "last_error": str(exc)},
            )
            await _publish_run_event(pipeline_id, {"type": "log", "msg": f"Pipeline Failed: {exc}"})
        finally:
            await _publish_lifecycle_phase(pipeline_id, PIPELINE_LIFECYCLE_PHASES[5])
            _pop_active_run(pipeline_id)
            await _publish_run_list_upsert(pipeline_id)

    asyncio.create_task(_run_retry())
    return {
        "status": "started",
        "pipeline_id": pipeline_id,
        "run_id": pipeline_id,
        "working_directory": working_dir,
        "model": display_model,
        "provider": selected_provider,
        "llm_provider": selected_provider,
        "reasoning_effort": selected_reasoning_effort,
        "diagnostics": [_diagnostic_payload(diagnostic) for diagnostic in diagnostics],
        "errors": [],
    }


async def _launch_pipeline_run(
    *,
    run_id: Optional[str],
    flow_name: str,
    flow_content: str,
    working_directory: str,
    backend_name: str,
    model: Optional[str],
    llm_provider: Optional[str] = None,
    reasoning_effort: Optional[str] = None,
    launch_context: Optional[dict[str, Any]] = None,
    spec_id: Optional[str] = None,
    plan_id: Optional[str] = None,
    flow_source_dir: Path | None = None,
    start_node_id: Optional[str] = None,
    on_complete: Optional[Callable[[str, str], Any]] = None,
    continued_from_run_id: Optional[str] = None,
    continued_from_node: Optional[str] = None,
    continued_from_flow_mode: Optional[str] = None,
    continued_from_flow_name: Optional[str] = None,
) -> dict:
    resolved_run_id = (run_id or uuid.uuid4().hex).strip()
    if not resolved_run_id:
        resolved_run_id = uuid.uuid4().hex
    if (
        _get_active_run(resolved_run_id) is not None
        or _read_run_meta(_run_meta_path(resolved_run_id)) is not None
        or _run_root_exists(resolved_run_id)
    ):
        return {
            "status": "validation_error",
            "error": f"Run id already exists: {resolved_run_id}",
        }
    run_root = _ensure_run_root_for_project(resolved_run_id, working_directory)
    await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[0])
    try:
        graph = parse_dot(flow_content)
    except DotParseError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.outcome = None
        RUNTIME.outcome_reason_code = None
        RUNTIME.outcome_reason_message = None
        RUNTIME.last_error = str(exc)
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
        }
        await _publish_run_event(resolved_run_id, {"type": "log", "msg": f"❌ Parse error: {exc}"})
        return {
            "status": "validation_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[1])
    graph, diagnostics = _prepare_graph_for_server(graph)
    await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[2])
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    diagnostic_payloads = [_diagnostic_payload(d) for d in diagnostics]
    error_payloads = [_diagnostic_payload(d) for d in errors]
    if errors:
        RUNTIME.status = "validation_error"
        RUNTIME.outcome = None
        RUNTIME.outcome_reason_code = None
        RUNTIME.outcome_reason_message = None
        RUNTIME.last_error = errors[0].message
        return {
            "status": "validation_error",
            "diagnostics": diagnostic_payloads,
            "errors": error_payloads,
        }

    resolved_start_node = (start_node_id or "").strip() or _resolve_start_node_id(graph)
    if resolved_start_node not in graph.nodes:
        return {
            "status": "validation_error",
            "error": f"Unknown start node: {resolved_start_node}",
            "diagnostics": diagnostic_payloads,
            "errors": error_payloads,
        }

    await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[3])

    os.makedirs(working_directory, exist_ok=True)
    working_dir = str(Path(working_directory).resolve())
    selected_model, display_model = _resolve_launch_model(graph, model)
    selected_provider = _resolve_launch_provider(graph, llm_provider)
    selected_reasoning_effort = _resolve_launch_reasoning_effort(graph, reasoning_effort)

    try:
        _validate_graph_provider_configuration(
            graph,
            launch_context=launch_context,
            selected_provider=selected_provider,
            selected_model=selected_model,
            selected_reasoning_effort=selected_reasoning_effort,
        )
    except ProviderConfigurationError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.outcome = None
        RUNTIME.outcome_reason_code = None
        RUNTIME.outcome_reason_message = None
        RUNTIME.last_error = str(exc)
        return {
            "status": "validation_error",
            "error": str(exc),
            "diagnostics": diagnostic_payloads,
            "errors": error_payloads,
        }

    await _publish_run_event(
        resolved_run_id,
        {
            "type": "graph",
            **_graph_payload(graph),
        },
    )

    loop = asyncio.get_running_loop()

    def emit(message: dict):
        asyncio.run_coroutine_threadsafe(_publish_run_event(resolved_run_id, message), loop)

    def handle_usage_update(token_usage_breakdown: TokenUsageBreakdown) -> None:
        _record_run_usage(resolved_run_id, token_usage_breakdown)
        _set_active_run_usage(resolved_run_id, token_usage_breakdown)
        asyncio.run_coroutine_threadsafe(_publish_run_list_upsert(resolved_run_id), loop)

    try:
        backend = _build_codergen_backend(
            backend_name,
            working_dir,
            emit,
            model=selected_model,
            on_usage_update=handle_usage_update,
        )
    except ValueError as exc:
        return {
            "status": "validation_error",
            "error": str(exc),
        }

    interviewer: Interviewer = WebInterviewer(HUMAN_BROKER, emit, flow_name, resolved_run_id)

    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )

    def launch_child_run(child_request: ChildRunRequest) -> ChildRunResult:
        return _run_first_class_child_pipeline(
            child_request,
            backend_name=backend_name,
            model=model,
            loop=loop,
        )

    runner = BroadcastingRunner(
        HandlerRunner(
            graph,
            registry,
            child_run_launcher=launch_child_run,
            child_status_resolver=_child_run_result_from_record,
        ),
        emit,
    )

    checkpoint_file = str(run_root / "state.json")
    logs_root = str(run_root / "logs")
    # NOTE: This artifact render intentionally uses the submitted DOT source.
    # It does not reflect transform/normalization changes applied to `graph`.
    # If post-transform fidelity is required, render from a serialized `graph` instead.
    graphviz_export = export_graphviz_artifact(flow_content, run_root)
    run_root.mkdir(parents=True, exist_ok=True)
    Path(logs_root).mkdir(parents=True, exist_ok=True)

    context = Context(values=dict(launch_context or {}))
    context.apply_updates(_graph_attr_context_seed(graph))
    context.set("current_node", resolved_start_node)
    context.set("outcome", "")
    context.set("preferred_label", "")
    context.set(RUNTIME_LAUNCH_MODEL_KEY, selected_model or "")
    context.set(RUNTIME_LAUNCH_PROVIDER_KEY, selected_provider)
    context.set(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, selected_reasoning_effort or "")
    context.set("internal.run_id", resolved_run_id)
    context.set("internal.root_run_id", resolved_run_id)
    context.set("internal.run_workdir", working_dir)
    if flow_source_dir is not None:
        context.set("internal.flow_source_dir", str(flow_source_dir))

    control = ExecutionControl()
    executor = PipelineExecutor(
        graph,
        runner,
        logs_root=logs_root,
        checkpoint_file=checkpoint_file,
        control=control.poll,
        on_event=emit,
    )
    if start_node_id is not None:
        executor._reset_workflow_outcome_context(context)
        executor._clear_runtime_retry_context(context)
        executor._mirror_graph_attrs(context)

    save_checkpoint(
        Path(checkpoint_file),
        Checkpoint(
            current_node=resolved_start_node,
            completed_nodes=[],
            context=dict(context.values),
            retry_counts={},
        ),
    )

    with ACTIVE_RUNS_LOCK:
        ACTIVE_RUNS[resolved_run_id] = ActiveRun(
            run_id=resolved_run_id,
            flow_name=flow_name,
            working_directory=working_dir,
            model=display_model,
            llm_provider=selected_provider,
            reasoning_effort=selected_reasoning_effort,
            status="running",
            control=control,
        )

    RUNTIME.status = "running"
    RUNTIME.outcome = None
    RUNTIME.outcome_reason_code = None
    RUNTIME.outcome_reason_message = None
    RUNTIME.last_error = ""
    RUNTIME.last_working_directory = working_dir
    RUNTIME.last_model = display_model
    RUNTIME.last_flow_name = flow_name

    _record_run_start(
        resolved_run_id,
        flow_name,
        working_dir,
        display_model,
        llm_provider=selected_provider,
        reasoning_effort=selected_reasoning_effort,
        spec_id=(spec_id or "").strip() or None,
        plan_id=(plan_id or "").strip() or None,
        continued_from_run_id=continued_from_run_id,
        continued_from_node=continued_from_node,
        continued_from_flow_mode=continued_from_flow_mode,
        continued_from_flow_name=continued_from_flow_name,
        root_run_id=resolved_run_id,
    )
    await _publish_run_list_upsert(resolved_run_id)

    await _publish_run_event(
        resolved_run_id,
        {
            "type": "runtime",
            "status": RUNTIME.status,
            "outcome": RUNTIME.outcome,
            "outcome_reason_code": RUNTIME.outcome_reason_code,
            "outcome_reason_message": RUNTIME.outcome_reason_message,
        },
    )

    await _publish_run_event(
        resolved_run_id,
        {
            "type": "run_meta",
            "working_directory": working_dir,
            "model": display_model,
            "provider": selected_provider,
            "llm_provider": selected_provider,
            "reasoning_effort": selected_reasoning_effort,
            "flow_name": flow_name,
            "run_id": resolved_run_id,
            "graph_source_path": str(graphviz_export.source_path),
            "graph_dot_path": str(graphviz_export.dot_path),
            "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
            "continued_from_run_id": continued_from_run_id,
            "continued_from_node": continued_from_node,
            "continued_from_flow_mode": continued_from_flow_mode,
            "continued_from_flow_name": continued_from_flow_name,
            "root_run_id": resolved_run_id,
        },
    )
    if graphviz_export.error:
        await _publish_run_event(
            resolved_run_id,
            {
                "type": "log",
                "msg": f"[System] Graph render unavailable: {graphviz_export.error}",
            },
        )
    await _publish_run_event(
        resolved_run_id,
        {
            "type": "log",
            "msg": f"[System] Launching run {resolved_run_id} in {working_dir} with model: {display_model}",
        },
    )

    async def _run():
        final_status = "failed"
        try:
            await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[4])
            if start_node_id is not None:
                result = await asyncio.to_thread(
                    executor.run_from,
                    resolved_start_node,
                    context,
                )
            else:
                result = await asyncio.to_thread(
                    executor.run,
                    context,
                    resume=True,
                )
            final_status = normalize_run_status(result.status)
            final_outcome = result.outcome
            final_outcome_reason_code = result.outcome_reason_code
            final_outcome_reason_message = result.outcome_reason_message
            final_last_error = ""
            if final_status in {"failed", "validation_error"}:
                final_last_error = (
                    str(result.failure_reason or "").strip()
                    or str(final_outcome_reason_message or "").strip()
                    or str(final_outcome_reason_code or "").strip()
                )
            _set_active_run_status(
                resolved_run_id,
                final_status,
                last_error=final_last_error if final_last_error else None,
            )
            _set_active_run_outcome(
                resolved_run_id,
                outcome=final_outcome,
                outcome_reason_code=final_outcome_reason_code,
                outcome_reason_message=final_outcome_reason_message,
            )
            _set_active_run_completed_nodes(resolved_run_id, result.completed_nodes)
            RUNTIME.status = final_status
            RUNTIME.outcome = final_outcome
            RUNTIME.outcome_reason_code = final_outcome_reason_code
            RUNTIME.outcome_reason_message = final_outcome_reason_message
            RUNTIME.last_error = final_last_error
            RUNTIME.last_completed_nodes = result.completed_nodes
            await _publish_run_event(
                resolved_run_id,
                {
                    "type": "runtime",
                    "status": final_status,
                    "outcome": final_outcome,
                    "outcome_reason_code": final_outcome_reason_code,
                    "outcome_reason_message": final_outcome_reason_message,
                    "last_error": final_last_error or None,
                },
            )
            _record_run_end(
                resolved_run_id,
                working_dir,
                final_status,
                final_last_error,
                outcome=final_outcome,
                outcome_reason_code=final_outcome_reason_code,
                outcome_reason_message=final_outcome_reason_message,
            )
            await _publish_run_list_upsert(resolved_run_id)
            await _publish_run_event(
                resolved_run_id,
                {
                    "type": "log",
                    "msg": _terminal_status_summary(
                        status=final_status,
                        outcome=final_outcome,
                        outcome_reason_code=final_outcome_reason_code,
                        outcome_reason_message=final_outcome_reason_message,
                        last_error=final_last_error,
                    ),
                },
            )
        except Exception as exc:  # noqa: BLE001
            import traceback
            traceback.print_exc()
            _set_active_run_status(resolved_run_id, "failed", last_error=str(exc))
            _set_active_run_outcome(
                resolved_run_id,
                outcome=None,
                outcome_reason_code=None,
                outcome_reason_message=None,
            )
            RUNTIME.status = "failed"
            RUNTIME.outcome = None
            RUNTIME.outcome_reason_code = None
            RUNTIME.outcome_reason_message = None
            RUNTIME.last_error = str(exc)
            await _publish_run_event(
                resolved_run_id,
                {
                    "type": "runtime",
                    "status": "failed",
                    "outcome": None,
                    "outcome_reason_code": None,
                    "outcome_reason_message": None,
                },
            )
            _record_run_end(resolved_run_id, working_dir, "failed", str(exc), outcome=None)
            await _publish_run_list_upsert(resolved_run_id)
            await _publish_run_event(resolved_run_id, {"type": "log", "msg": f"⚠️ Pipeline Failed: {exc}"})
        finally:
            await _publish_lifecycle_phase(resolved_run_id, PIPELINE_LIFECYCLE_PHASES[5])
            _pop_active_run(resolved_run_id)
            if on_complete is not None:
                try:
                    completion_result = on_complete(resolved_run_id, final_status)
                    if asyncio.iscoroutine(completion_result):
                        await completion_result
                except Exception:  # noqa: BLE001
                    LOGGER.exception("pipeline completion callback failed for run %s", resolved_run_id)

    asyncio.create_task(_run())
    return {
        "status": "started",
        "pipeline_id": resolved_run_id,
        "run_id": resolved_run_id,
        "working_directory": working_dir,
        "model": display_model,
        "provider": selected_provider,
        "llm_provider": selected_provider,
        "reasoning_effort": selected_reasoning_effort,
        "diagnostics": diagnostic_payloads,
        "errors": error_payloads,
        "graph_dot_path": str(graphviz_export.dot_path),
        "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
    }


async def _start_pipeline(
    req: PipelineStartRequest,
    *,
    run_id: Optional[str] = None,
    on_complete: Optional[Callable[[str, str], Any]] = None,
) -> dict:
    flow_name = (req.flow_name or "").strip()
    flow_content = (req.flow_content or "").strip()
    if not flow_content:
        if not flow_name:
            return {
                "status": "validation_error",
                "error": "Either flow_content or flow_name is required.",
            }
        try:
            flow_content = _load_flow_content(flow_name)
        except HTTPException as exc:
            return {
                "status": "validation_error" if exc.status_code == 400 else "failed",
                "error": str(exc.detail),
            }
    if req.goal:
        flow_content = _inject_pipeline_goal_impl(flow_content, req.goal)
    flow_source_dir: Path | None = None
    if flow_name:
        try:
            flow_path = _resolve_flow_path(flow_name)
        except HTTPException:
            flow_path = None
        if flow_path is not None and flow_path.exists():
            flow_source_dir = flow_path.parent.resolve()

    try:
        launch_context = normalize_launch_context(
            req.launch_context,
            source_name="Attractor pipeline start",
        )
    except ValueError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.outcome = None
        RUNTIME.outcome_reason_code = None
        RUNTIME.outcome_reason_message = None
        RUNTIME.last_error = str(exc)
        return {
            "status": "validation_error",
            "error": str(exc),
        }

    return await _launch_pipeline_run(
        run_id=run_id,
        flow_name=flow_name,
        flow_content=flow_content,
        working_directory=req.working_directory,
        backend_name="provider-router",
        model=req.model,
        llm_provider=req.llm_provider,
        reasoning_effort=req.reasoning_effort,
        launch_context=launch_context,
        spec_id=req.spec_id,
        plan_id=req.plan_id,
        flow_source_dir=flow_source_dir,
        on_complete=on_complete,
    )


@attractor_router.post("/pipelines")
async def create_pipeline(req: PipelineStartRequest):
    return await _start_pipeline(req, run_id=req.run_id)


@attractor_router.post("/pipelines/{pipeline_id}/continue")
async def continue_pipeline(pipeline_id: str, req: PipelineContinueRequest):
    source_record, source_checkpoint = _resolve_continue_source_record(pipeline_id)

    try:
        flow_name, flow_content, flow_source_dir = _resolve_continue_flow_source(
            pipeline_id=pipeline_id,
            source_record=source_record,
            flow_source_mode=req.flow_source_mode,
            flow_name=req.flow_name,
        )
    except ValueError as exc:
        return {
            "status": "validation_error",
            "error": str(exc),
        }
    except HTTPException as exc:
        return {
            "status": "validation_error" if exc.status_code == 400 else "failed",
            "error": str(exc.detail),
        }

    start_node = (req.start_node or "").strip()
    if not start_node:
        return {
            "status": "validation_error",
            "error": "start_node is required.",
        }

    working_directory = (req.working_directory or "").strip() or source_record.working_directory
    source_context = dict(source_checkpoint.context)
    model = req.model if req.model is not None else (
        str(source_context.get(RUNTIME_LAUNCH_MODEL_KEY, "")).strip() or source_record.model
    )
    llm_provider = req.llm_provider
    if llm_provider is None:
        llm_provider = str(source_context.get(RUNTIME_LAUNCH_PROVIDER_KEY, "")).strip() or None
    reasoning_effort = req.reasoning_effort
    if reasoning_effort is None:
        reasoning_effort = str(source_context.get(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, "")).strip() or None

    return await _launch_pipeline_run(
        run_id=None,
        flow_name=flow_name,
        flow_content=flow_content,
        working_directory=working_directory,
        backend_name="provider-router",
        model=model,
        llm_provider=llm_provider,
        reasoning_effort=reasoning_effort,
        launch_context=source_context,
        spec_id=source_record.spec_id,
        plan_id=source_record.plan_id,
        flow_source_dir=flow_source_dir,
        start_node_id=start_node,
        continued_from_run_id=source_record.run_id,
        continued_from_node=start_node,
        continued_from_flow_mode=req.flow_source_mode.strip().lower(),
        continued_from_flow_name=(req.flow_name or "").strip() or None,
    )


@attractor_router.post("/pipelines/{pipeline_id}/retry")
async def retry_pipeline(pipeline_id: str):
    return await _retry_pipeline_run(pipeline_id)


@attractor_router.get("/pipelines/{pipeline_id}")
async def get_pipeline(pipeline_id: str):
    return _pipeline_status_payload(pipeline_id)


@attractor_router.get("/pipelines/{pipeline_id}/checkpoint")
async def get_pipeline_checkpoint(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)

    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=404, detail="Checkpoint unavailable")

    return {
        "pipeline_id": pipeline_id,
        "checkpoint": checkpoint.to_dict(),
    }


@attractor_router.get("/pipelines/{pipeline_id}/context")
async def get_pipeline_context(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)

    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=404, detail="Context unavailable")

    return {
        "pipeline_id": pipeline_id,
        "context": dict(checkpoint.context),
    }


@attractor_router.patch("/pipelines/{pipeline_id}/metadata")
async def update_pipeline_metadata(pipeline_id: str, req: PipelineMetadataUpdateRequest):
    _ensure_known_pipeline(pipeline_id)
    await asyncio.to_thread(
        _record_run_metadata,
        pipeline_id,
        spec_id=(req.spec_id or "").strip() or None,
        plan_id=(req.plan_id or "").strip() or None,
    )
    await _publish_run_list_upsert(pipeline_id)
    record = _read_run_meta(_run_meta_path(pipeline_id))
    if record is None:
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    return record.to_dict()


@attractor_router.get("/pipelines/{pipeline_id}/artifacts")
async def list_pipeline_artifacts(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)
    run_root = _run_root(pipeline_id)
    return {
        "pipeline_id": pipeline_id,
        "artifacts": _list_run_output_artifacts(run_root),
    }


@attractor_router.get("/pipelines/{pipeline_id}/artifacts/{artifact_path:path}")
async def get_pipeline_artifact_file(pipeline_id: str, artifact_path: str, download: bool = False):
    _ensure_known_pipeline(pipeline_id)
    run_root = _run_root(pipeline_id)
    resolved_artifact_path = _resolve_artifact_path(run_root, artifact_path)
    if not resolved_artifact_path.is_file():
        raise HTTPException(status_code=404, detail="Artifact not found")

    media_type = _artifact_media_type(resolved_artifact_path)
    if download:
        return FileResponse(
            resolved_artifact_path,
            media_type=media_type,
            filename=resolved_artifact_path.name,
        )
    return FileResponse(resolved_artifact_path, media_type=media_type)


@attractor_router.get("/pipelines/{pipeline_id}/journal")
async def pipeline_journal(
    pipeline_id: str,
    limit: int = DEFAULT_RUN_JOURNAL_PAGE_SIZE,
    before_sequence: int | None = None,
):
    _ensure_known_pipeline(pipeline_id)
    normalized_limit = _normalize_run_journal_page_limit(limit)
    normalized_before_sequence = _normalize_before_sequence(before_sequence)
    entries = _run_journal_entries(pipeline_id)
    if normalized_before_sequence is not None:
        entries = [entry for entry in entries if entry["sequence"] < normalized_before_sequence]
    page = entries[:normalized_limit]
    return {
        "pipeline_id": pipeline_id,
        "entries": page,
        "oldest_sequence": page[-1]["sequence"] if page else None,
        "newest_sequence": page[0]["sequence"] if page else None,
        "has_older": len(entries) > len(page),
    }


@attractor_router.get("/pipelines/{pipeline_id}/events")
async def pipeline_events(
    pipeline_id: str,
    request: Request,
    after_sequence: int | None = None,
):
    normalized_after_sequence = _normalize_after_sequence(after_sequence)
    active = _get_active_run(pipeline_id)
    existing = _read_run_meta(_run_meta_path(pipeline_id))
    queue = EVENT_HUB.subscribe(pipeline_id)
    try:
        persisted_history = _read_persisted_run_events(pipeline_id)
        live_history = EVENT_HUB.history(pipeline_id)
        if not active and not existing and not persisted_history and not live_history:
            raise HTTPException(status_code=404, detail="Unknown pipeline")
    except Exception:
        EVENT_HUB.unsubscribe(pipeline_id, queue)
        raise

    async def stream():
        highest_sequence = normalized_after_sequence or 0
        try:
            if normalized_after_sequence is not None:
                gap_fill_entries: list[dict[str, Any]] = []
                seen_sequences: set[int] = set()
                for event in persisted_history + live_history:
                    entry = _run_journal_entry_from_event(event)
                    if entry is None:
                        continue
                    sequence = entry["sequence"]
                    if sequence <= normalized_after_sequence or sequence in seen_sequences:
                        continue
                    seen_sequences.add(sequence)
                    gap_fill_entries.append(entry)
                gap_fill_entries.sort(key=lambda item: item["sequence"])
                for entry in gap_fill_entries:
                    highest_sequence = max(highest_sequence, entry["sequence"])
                    yield f"data: {json.dumps(entry)}\n\n"

            while True:
                if await request.is_disconnected():
                    break
                try:
                    event = await asyncio.wait_for(queue.get(), timeout=15.0)
                    entry = _run_journal_entry_from_event(event)
                    if entry is None:
                        continue
                    sequence = entry["sequence"]
                    if sequence <= highest_sequence:
                        continue
                    highest_sequence = sequence
                    yield f"data: {json.dumps(entry)}\n\n"
                except asyncio.TimeoutError:
                    yield ": keepalive\n\n"
        finally:
            EVENT_HUB.unsubscribe(pipeline_id, queue)

    return StreamingResponse(
        stream(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    )


@attractor_router.post("/pipelines/{pipeline_id}/cancel")
async def cancel_pipeline(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active:
        if not _read_run_meta(_run_meta_path(pipeline_id)):
            raise HTTPException(status_code=404, detail="Unknown pipeline")
        return {"status": "ignored", "pipeline_id": pipeline_id}
    active.control.request_cancel()
    _set_active_run_status(pipeline_id, "cancel_requested", last_error="cancel_requested_by_user")
    _record_run_status(pipeline_id, "cancel_requested", "cancel_requested_by_user")
    await _publish_run_list_upsert(pipeline_id)
    RUNTIME.status = "cancel_requested"
    RUNTIME.outcome = None
    RUNTIME.outcome_reason_code = None
    RUNTIME.outcome_reason_message = None
    RUNTIME.last_error = "cancel_requested_by_user"
    await _publish_run_event(
        pipeline_id,
        {
            "type": "runtime",
            "status": "cancel_requested",
            "outcome": None,
            "outcome_reason_code": None,
            "outcome_reason_message": None,
        },
    )
    await _publish_run_event(
        pipeline_id,
        {"type": "log", "msg": "[System] Cancel requested. Stopping after current node."},
    )
    return {"status": "cancel_requested", "pipeline_id": pipeline_id}


@attractor_router.get("/pipelines/{pipeline_id}/graph")
async def get_pipeline_graph(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    graph_svg_path = _run_root(pipeline_id) / "artifacts" / "graphviz" / "pipeline.svg"
    if not graph_svg_path.exists():
        raise HTTPException(status_code=404, detail="Graph visualization unavailable")

    return FileResponse(graph_svg_path, media_type="image/svg+xml")


@attractor_router.get("/pipelines/{pipeline_id}/graph-preview")
async def get_pipeline_graph_preview(pipeline_id: str, expand_children: bool = False):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    graph_source_path = _resolve_run_graph_source_path(pipeline_id)
    flow_source_dir, run_workdir = _resolve_run_preview_source_context(pipeline_id)
    return _preview_payload_from_dot_source(
        graph_source_path.read_text(encoding="utf-8"),
        expand_children=expand_children,
        flow_source_dir=flow_source_dir,
        run_workdir=run_workdir,
    )


@attractor_router.get("/pipelines/{pipeline_id}/questions")
async def list_pipeline_questions(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    return {"questions": HUMAN_BROKER.list_for_run(pipeline_id)}


@attractor_router.post("/pipelines/{pipeline_id}/questions/{question_id}/answer")
async def submit_pipeline_answer(pipeline_id: str, question_id: str, req: HumanAnswerRequest):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    ok = HUMAN_BROKER.answer(pipeline_id, question_id, req.selected_value)
    if not ok:
        raise HTTPException(status_code=404, detail="Unknown question for pipeline")
    return {"status": "accepted", "pipeline_id": pipeline_id, "question_id": question_id}


@attractor_router.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    runs_root = get_runtime_paths().runs_dir
    if runs_root.exists():
        shutil.rmtree(runs_root, ignore_errors=True)
        runs_root.mkdir(parents=True, exist_ok=True)
    return {"status": "reset"}


def _resolve_project_git_branch(directory_path: Path) -> Optional[str]:
    return pipeline_runs.resolve_project_git_branch(directory_path)


def _resolve_project_git_commit(directory_path: Path) -> Optional[str]:
    return pipeline_runs.resolve_project_git_commit(directory_path)


def _resolve_run_project_git_metadata(working_directory: str) -> tuple[str, Optional[str], Optional[str]]:
    return pipeline_runs.resolve_run_project_git_metadata(
        working_directory,
        resolve_runtime_workspace_path=resolve_runtime_workspace_path,
    )


def _load_flow_content(flow_source: str) -> str:
    return _load_flow_content_impl(_flows_dir(), flow_source)
def _record_run_metadata(run_id: str, *, spec_id: Optional[str] = None, plan_id: Optional[str] = None) -> None:
    with RUN_HISTORY_LOCK:
        record = _read_run_meta(_run_meta_path(run_id))
        if record is None:
            raise HTTPException(status_code=404, detail="Unknown pipeline")
        if spec_id is not None:
            record.spec_id = spec_id
        if plan_id is not None:
            record.plan_id = plan_id
        _write_run_meta(record)


@attractor_router.get("/api/flows")
async def list_flows():
    flows_dir = _flows_dir()
    flow_paths = sorted(
        (path for path in flows_dir.rglob("*.dot") if path.is_file()),
        key=lambda path: _flow_name_from_path_impl(flows_dir, path),
    )
    return [_flow_name_from_path_impl(flows_dir, flow_path) for flow_path in flow_paths]


def _flows_dir() -> Path:
    return _ensure_flows_dir_impl(get_runtime_paths().flows_dir)


def _resolve_flow_path(flow_name: str) -> Path:
    return _resolve_flow_path_impl(_flows_dir(), flow_name)


@attractor_router.get("/api/flows/{name:path}")
async def get_flow(name: str):
    flow_path = _resolve_flow_path(name)
    if not flow_path.exists():
        raise HTTPException(status_code=404, detail="Flow not found.")
    return {
        "name": _flow_name_from_path_impl(_flows_dir(), flow_path),
        "content": flow_path.read_text(encoding="utf-8"),
    }


def _semantic_signature(dot_content: str) -> str:
    return _semantic_signature_impl(dot_content, _build_transform_pipeline)


@attractor_router.post("/api/flows")
async def save_flow(req: SaveFlowRequest):
    canonical_content: str
    try:
        graph = parse_dot(req.content)
        canonical_content = _canonicalize_graph_source(req.content)
    except DotParseError as exc:
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
            "node_id": None,
        }
        raise HTTPException(
            status_code=422,
            detail={
                "status": "parse_error",
                "error": f"invalid DOT: {exc}",
                "diagnostics": [parse_diag],
                "errors": [parse_diag],
            },
        ) from exc

    graph, diagnostics = _prepare_graph_for_server(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    if errors:
        diagnostic_payloads = [_diagnostic_payload(d) for d in diagnostics]
        error_payloads = [_diagnostic_payload(d) for d in errors]
        raise HTTPException(
            status_code=422,
            detail={
                "status": "validation_error",
                "error": "validation errors prevent saving this flow",
                "diagnostics": diagnostic_payloads,
                "errors": error_payloads,
            },
        )

    flow_path = _resolve_flow_path(req.name)

    semantic_equivalent_to_existing: bool | None = None
    if flow_path.exists():
        try:
            semantic_equivalent_to_existing = _semantic_signature(flow_path.read_text()) == _semantic_signature(req.content)
        except DotParseError:
            semantic_equivalent_to_existing = None

        if req.expect_semantic_equivalence and semantic_equivalent_to_existing is False:
            raise HTTPException(
                status_code=409,
                detail={
                    "status": "semantic_mismatch",
                    "error": "semantic equivalence check failed: output DOT would change flow behavior",
                },
            )

    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text(canonical_content, encoding="utf-8")
    response: Dict[str, object] = {
        "status": "saved",
        "name": _flow_name_from_path_impl(_flows_dir(), flow_path),
    }
    if semantic_equivalent_to_existing is not None:
        response["semantic_equivalent_to_existing"] = semantic_equivalent_to_existing
    return response


@attractor_router.delete("/api/flows/{flow_name:path}")
async def delete_flow(flow_name: str):
    filepath = _resolve_flow_path(flow_name)
    if filepath.exists():
        filepath.unlink()
        return {"status": "deleted"}
    raise HTTPException(status_code=404, detail="Flow not found.")


attractor_app.router.lifespan_context = _attractor_lifespan
attractor_app.include_router(attractor_router)
