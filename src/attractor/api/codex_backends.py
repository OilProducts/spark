from __future__ import annotations

from contextlib import contextmanager
from datetime import UTC, datetime
import json
import queue
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Callable, Mapping, Optional
import uuid

from attractor.api.token_usage import (
    TokenUsageBreakdown,
    TokenUsageBucket,
    compute_live_usage_delta,
)
from attractor.api.codergen_contracts import (
    ModeledOutcomeParseResult as _ModeledOutcomeParseResult,
    PlainTextParseResult as _PlainTextParseResult,
    StructuredContractViolation as _StructuredContractViolation,
    build_contract_repair_prompt as _build_contract_repair_prompt,
    coerce_structured_text_outcome as _coerce_structured_text_outcome,
    contract_failure_outcome as _contract_failure_outcome,
    has_response_contract as _has_response_contract,
    validate_write_contract_violation as _validate_write_contract_violation,
    with_write_contract as _with_write_contract,
)
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers.base import ChildInterventionRequest, ChildInterventionResult, CodergenBackend
from spark.chat.session import (
    RustAgentBoundary,
    RustBoundaryError,
    SerializedRustAgentBoundary,
    normalize_boundary_provider_selector,
    token_usage_payload_from_boundary_usage,
    turn_stream_event_from_boundary_payload,
)
from spark.workspace.conversations.utils import (
    as_non_empty_string,
    normalize_project_path_value,
)
from spark_common.codex_app_client import CodexAppServerClient
from spark_common import codex_app_protocol
from spark_common.turn_stream import TurnStreamEvent
from spark_common.runtime_path import resolve_runtime_workspace_path

UNIFIED_AGENT_PROVIDERS = {"openai", "anthropic", "gemini", "openrouter", "litellm", "openai_compatible"}
SUPPORTED_LLM_PROVIDERS = {"codex", *UNIFIED_AGENT_PROVIDERS}
SUPPORTED_LLM_PROVIDER_MESSAGE = (
    "Supported providers: codex, openai, anthropic, gemini, openrouter, litellm, openai_compatible."
)


def _rejected_intervention(
    request: ChildInterventionRequest,
    *,
    reason: str,
    delivery_mode: str = "none",
    message: str = "",
) -> ChildInterventionResult:
    return ChildInterventionResult(
        run_id=request.child_run_id,
        status="rejected",
        delivery_mode=delivery_mode,
        reason=reason,
        message=message,
        target_node_id=request.target_node_id,
    )


def _delivered_intervention(
    request: ChildInterventionRequest,
    *,
    delivery_mode: str,
    message: str = "",
) -> ChildInterventionResult:
    return ChildInterventionResult(
        run_id=request.child_run_id,
        status="delivered",
        delivery_mode=delivery_mode,
        reason=request.reason,
        message=message,
        target_node_id=request.target_node_id,
    )


def _turn_stream_source_payload(event: TurnStreamEvent) -> dict[str, object]:
    payload: dict[str, object] = {}
    for key in (
        "backend",
        "session_id",
        "app_turn_id",
        "item_id",
        "response_id",
        "summary_index",
        "raw_kind",
    ):
        value = getattr(event.source, key)
        if value is not None:
            payload[key] = value
    return payload


def _emit_turn_stream_progress(
    emit_event: Optional[Callable[..., None]],
    *,
    node_id: str,
    event: TurnStreamEvent,
) -> None:
    if emit_event is None:
        return
    if event.kind not in {"content_delta", "content_completed"}:
        return
    if event.channel not in {"assistant", "reasoning", "plan"}:
        return
    content = str(event.content_delta or "")
    if not content:
        return
    emit_event(
        "LLMContent",
        node_id=node_id,
        channel=event.channel,
        content_delta=content,
        status="complete" if event.kind == "content_completed" else "streaming",
        phase=event.phase,
        source=_turn_stream_source_payload(event),
    )


def _is_provider_setup_failure(reason: str) -> bool:
    text = str(reason or "").strip().lower()
    if not text:
        return False
    non_retryable_tokens = (
        "api key",
        "auth",
        "credential",
        "permission",
        "forbidden",
        "unauthorized",
        "configuration",
        "config",
        "not configured",
        "missing",
        "not found on path",
        "unsupported llm_provider",
        "thread/start failed",
    )
    return any(token in text for token in non_retryable_tokens)


class CodexAppServerBackend(CodergenBackend):
    RUNTIME_THREAD_ID_KEY = "_attractor.runtime.thread_id"

    def __init__(
        self,
        working_dir: str,
        emit,
        model: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
    ):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.model = model
        self._on_usage_update = on_usage_update
        self._session_threads_by_key: dict[str, str] = {}
        self._session_threads_lock = threading.Lock()
        self._raw_rpc_log_lock = threading.Lock()
        self._raw_rpc_log_state = threading.local()
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()
        self._active_turn_lock = threading.Lock()
        self._active_client: CodexAppServerClient | None = None
        self._active_thread_id: str | None = None
        self._active_turn_id: str | None = None

    @contextmanager
    def bind_stage_raw_rpc_log(self, node_id: str, logs_root: str | Path | None):
        previous = getattr(self._raw_rpc_log_state, "path", None)
        self._raw_rpc_log_state.path = self._stage_raw_rpc_log_path(node_id, logs_root)
        try:
            yield
        finally:
            if previous is None:
                if hasattr(self._raw_rpc_log_state, "path"):
                    delattr(self._raw_rpc_log_state, "path")
            else:
                self._raw_rpc_log_state.path = previous

    def _stage_raw_rpc_log_path(self, node_id: str, logs_root: str | Path | None) -> Path | None:
        if logs_root is None:
            return None
        stage_dir = Path(logs_root) / node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        return stage_dir / "raw-rpc.jsonl"

    def _append_raw_rpc_log(self, direction: str, line: str) -> None:
        path = getattr(self._raw_rpc_log_state, "path", None)
        if path is None:
            return
        payload = {
            "timestamp": datetime.now(UTC).isoformat(),
            "direction": direction,
            "line": line,
        }
        with self._raw_rpc_log_lock:
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(payload, sort_keys=True) + "\n")

    def _runtime_thread_key(self, context: Context) -> str:
        value = context.get(self.RUNTIME_THREAD_ID_KEY, "")
        if value is None:
            return ""
        return str(value).strip()

    def _resolve_session_thread_id(
        self,
        thread_key: str,
        model: Optional[str],
        start_thread: Callable[[], str | None],
    ) -> str | None:
        normalized_key = thread_key.strip()
        if not normalized_key:
            return start_thread()

        cache_key = normalized_key
        normalized_model = str(model or "").strip()
        if normalized_model:
            cache_key = f"{normalized_key}::{normalized_model}"

        with self._session_threads_lock:
            cached = self._session_threads_by_key.get(cache_key)
            if cached:
                return cached
            created = start_thread()
            if not created:
                return None
            self._session_threads_by_key[cache_key] = created
            return created

    def _record_token_usage_delta(self, *, model: Optional[str], delta: TokenUsageBucket) -> None:
        if not delta.has_any_usage():
            return
        normalized_model = str(model or "").strip() or "codex default (config/profile)"
        with self._token_usage_lock:
            self._token_usage_breakdown.add_for_model(normalized_model, delta)
            snapshot = self._token_usage_breakdown.copy()
        if self._on_usage_update is not None:
            self._on_usage_update(snapshot)

    def _set_active_turn(
        self,
        *,
        client: CodexAppServerClient,
        thread_id: str,
        turn_id: str,
    ) -> None:
        with self._active_turn_lock:
            self._active_client = client
            self._active_thread_id = thread_id
            self._active_turn_id = turn_id

    def _clear_active_turn(self, *, client: CodexAppServerClient) -> None:
        with self._active_turn_lock:
            if self._active_client is not client:
                return
            self._active_client = None
            self._active_thread_id = None
            self._active_turn_id = None

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        with self._active_turn_lock:
            client = self._active_client
            thread_id = self._active_thread_id
            turn_id = self._active_turn_id
        if client is None or not thread_id or not turn_id:
            return _rejected_intervention(
                request,
                reason="no_active_turn",
                message="No active codex app-server turn is available for intervention.",
            )
        try:
            client.steer_turn(thread_id, turn_id, request.message)
        except RuntimeError as exc:
            return _rejected_intervention(
                request,
                reason="app_server_steer_failed",
                delivery_mode="codex_app_server_turn",
                message=str(exc),
            )
        return _delivered_intervention(
            request,
            delivery_mode="codex_app_server_turn",
            message="Intervention delivered to active codex app-server turn.",
        )

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: Optional[float] = None,
        model: Optional[str] = None,
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        del provider
        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                retryable=False if _is_provider_setup_failure(reason) else None,
                failure_kind=FailureKind.RUNTIME,
            )

        client = CodexAppServerClient(
            self.working_dir,
            requested_working_dir=self.requested_working_dir,
            on_unparsed_line=log_line,
        )
        effective_model = str(model or "").strip() or self.model
        set_raw_rpc_logger = getattr(client, "set_raw_rpc_logger", None)
        clear_raw_rpc_logger = getattr(client, "clear_raw_rpc_logger", None)
        try:
            if callable(set_raw_rpc_logger):
                set_raw_rpc_logger(self._append_raw_rpc_log)
            client.ensure_process(popen_factory=subprocess.Popen)

            def start_thread() -> str | None:
                try:
                    return client.start_thread(
                        model=effective_model,
                        cwd=self.working_dir,
                        ephemeral=True,
                    )
                except RuntimeError:
                    return None

            thread_key = self._runtime_thread_key(context)
            thread_uuid = self._resolve_session_thread_id(thread_key, effective_model, start_thread)
            if not thread_uuid:
                return fail("codex app-server thread/start failed")

            turn_text = self._run_turn_and_capture_text(
                client,
                thread_uuid,
                prompt,
                timeout,
                log_line,
                model=effective_model,
                reasoning_effort=reasoning_effort,
                node_id=node_id,
                emit_event=emit_event,
            )
            if turn_text is None:
                return "codex app-server completed successfully"
            return self._coerce_or_repair_contract_result(
                client,
                thread_uuid,
                node_id,
                turn_text,
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                timeout=timeout,
                log_line=log_line,
                model=effective_model,
                reasoning_effort=reasoning_effort,
                write_contract=write_contract,
                emit_event=emit_event,
            )
        except RuntimeError as exc:
            return fail(str(exc))
        finally:
            if callable(clear_raw_rpc_logger):
                clear_raw_rpc_logger()
            client.close()

    def _run_turn_and_capture_text(
        self,
        client: CodexAppServerClient,
        thread_id: str,
        prompt: str,
        timeout: Optional[float],
        log_line: Callable[[str], None],
        *,
        model: Optional[str],
        reasoning_effort: Optional[str],
        node_id: str,
        emit_event: Optional[Callable[..., None]] = None,
    ) -> str | None:
        previous_total: TokenUsageBucket | None = None
        saw_usage_update = False

        def handle_turn_event(event: TurnStreamEvent) -> None:
            nonlocal previous_total, saw_usage_update
            _emit_turn_stream_progress(emit_event, node_id=node_id, event=event)
            if event.kind != "token_usage_updated" or event.token_usage is None:
                return
            delta, previous_total = compute_live_usage_delta(event.token_usage, previous_total)
            if delta is None or not delta.has_any_usage():
                return
            saw_usage_update = True
            self._record_token_usage_delta(model=model, delta=delta)

        try:
            result = client.run_turn(
                thread_id=thread_id,
                prompt=prompt,
                model=model,
                reasoning_effort=reasoning_effort,
                cwd=self.working_dir,
                on_event=handle_turn_event,
                on_turn_started=lambda turn_id: self._set_active_turn(
                    client=client,
                    thread_id=thread_id,
                    turn_id=turn_id,
                ),
                overall_timeout_seconds=timeout,
                now=time.monotonic,
            )
        finally:
            self._clear_active_turn(client=client)
        if not saw_usage_update:
            delta, _ = compute_live_usage_delta(getattr(result, "token_usage_payload", None), None)
            if delta is not None and delta.has_any_usage():
                self._record_token_usage_delta(model=model, delta=delta)
        agent_text = result.assistant_message
        if agent_text:
            log_line(agent_text)
        command_text = result.command_text
        if command_text:
            log_line(command_text)
        if result.token_total is not None:
            log_line(f"tokens used: {result.token_total}")
        return agent_text or command_text or None

    def _coerce_or_repair_contract_result(
        self,
        client: CodexAppServerClient,
        thread_id: str,
        node_id: str,
        response_text: str,
        *,
        response_contract: str,
        contract_repair_attempts: int,
        timeout: Optional[float],
        log_line: Callable[[str], None],
        model: Optional[str],
        reasoning_effort: Optional[str],
        write_contract: ContextWriteContract | None,
        emit_event: Optional[Callable[..., None]] = None,
    ) -> str | Outcome:
        result = _coerce_structured_text_outcome(response_text, response_contract=response_contract)
        if isinstance(result, Outcome):
            return result
        if isinstance(result, _ModeledOutcomeParseResult):
            violation = _validate_write_contract_violation(
                result.outcome,
                write_contract=write_contract,
                response_contract=response_contract,
                raw_text=response_text,
            )
            if violation is None:
                return result.outcome
            result = violation
        if isinstance(result, _PlainTextParseResult):
            return result.raw_text

        if contract_repair_attempts <= 0:
            return _contract_failure_outcome(result)

        current_violation = _with_write_contract(result, write_contract)
        for attempt in range(1, contract_repair_attempts + 1):
            log_line(
                f"response contract violation for {node_id}; requesting corrected final answer "
                f"(attempt {attempt}/{contract_repair_attempts}): {current_violation.reason}"
            )
            repair_prompt = _build_contract_repair_prompt(current_violation)
            repair_text = self._run_turn_and_capture_text(
                client,
                thread_id,
                repair_prompt,
                timeout,
                log_line,
                model=model,
                reasoning_effort=reasoning_effort,
                node_id=node_id,
                emit_event=emit_event,
            )
            if repair_text is None:
                return _contract_failure_outcome(current_violation)
            repaired = _coerce_structured_text_outcome(
                repair_text,
                response_contract=current_violation.response_contract,
            )
            if isinstance(repaired, _ModeledOutcomeParseResult):
                repaired_violation = _validate_write_contract_violation(
                    repaired.outcome,
                    write_contract=write_contract,
                    response_contract=current_violation.response_contract,
                    raw_text=repair_text,
                )
                if repaired_violation is None:
                    return repaired.outcome
                current_violation = repaired_violation
                continue
            if isinstance(repaired, _PlainTextParseResult):
                return repaired.raw_text
            current_violation = _with_write_contract(repaired, write_contract)
        return _contract_failure_outcome(current_violation)


def _normalize_provider(value: Optional[str]) -> str:
    try:
        return normalize_boundary_provider_selector(value)
    except ValueError as exc:
        raise ValueError(f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}") from exc


def _breakdown_delta_from(
    current: TokenUsageBreakdown,
    previous: TokenUsageBreakdown | None,
) -> TokenUsageBreakdown:
    delta = TokenUsageBreakdown()
    if current.by_model:
        for model_id, usage in current.by_model.items():
            previous_usage = previous.by_model.get(model_id) if previous is not None else None
            model_delta = usage.delta_from(previous_usage or TokenUsageBucket())
            if model_delta.has_any_usage():
                delta.add_for_model(model_id, model_delta)
        return delta

    current_total = TokenUsageBucket(
        input_tokens=current.input_tokens,
        cached_input_tokens=current.cached_input_tokens,
        output_tokens=current.output_tokens,
        total_tokens=current.total_tokens,
    )
    previous_total = TokenUsageBucket(
        input_tokens=previous.input_tokens,
        cached_input_tokens=previous.cached_input_tokens,
        output_tokens=previous.output_tokens,
        total_tokens=previous.total_tokens,
    ) if previous is not None else TokenUsageBucket()
    aggregate_delta = current_total.delta_from(previous_total)
    if aggregate_delta.has_any_usage():
        delta.add_for_model("unknown", aggregate_delta)
    return delta


def _context_snapshot(context: Context) -> dict[str, Any]:
    snapshot = context.snapshot()
    return snapshot if isinstance(snapshot, dict) else {}


def _write_contract_payload(write_contract: ContextWriteContract | None) -> dict[str, Any]:
    if write_contract is None:
        return {"allowed_keys": [], "parse_error": ""}
    return {
        "allowed_keys": list(write_contract.allowed_keys),
        "parse_error": write_contract.parse_error,
    }


def build_codergen_backend_request_payload(
    *,
    node_id: str,
    prompt: str,
    context: Mapping[str, Any],
    response_contract: str,
    contract_repair_attempts: int,
    timeout: Optional[float],
    write_contract: ContextWriteContract | None,
    provider: str | None,
    model: str | None,
    llm_profile: str | None,
    reasoning_effort: str | None,
    project_path: str,
    metadata: Mapping[str, Any] | None = None,
    repair_attempt: int | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "node_id": str(node_id or ""),
        "prompt": str(prompt or ""),
        "context": dict(context),
        "response_contract": str(response_contract or ""),
        "contract_repair_attempts": max(0, int(contract_repair_attempts or 0)),
        "write_contract": _write_contract_payload(write_contract),
        "provider": _normalize_provider(provider),
        "project_path": normalize_project_path_value(project_path),
        "metadata": dict(metadata or {}),
    }
    if timeout is not None:
        payload["timeout_seconds"] = float(timeout)
    model_id = as_non_empty_string(model)
    profile_id = as_non_empty_string(llm_profile)
    effort = as_non_empty_string(reasoning_effort)
    if model_id is not None:
        payload["model"] = model_id
    if profile_id is not None:
        payload["llm_profile"] = profile_id
    if effort is not None:
        payload["reasoning_effort"] = effort.lower()
    if repair_attempt is not None:
        payload["repair_attempt"] = max(0, int(repair_attempt))
    return payload


def _usage_bucket_from_boundary_payload(payload: Any) -> TokenUsageBucket:
    if not isinstance(payload, Mapping):
        return TokenUsageBucket()
    direct_bucket = TokenUsageBucket.from_dict(payload)
    if direct_bucket is not None:
        return direct_bucket
    total = payload.get("total")
    total_bucket = TokenUsageBucket.from_dict(total if isinstance(total, Mapping) else None)
    return total_bucket or TokenUsageBucket()


def _boundary_token_usage_payload(output: Mapping[str, Any]) -> Mapping[str, Any] | None:
    token_usage = output.get("token_usage")
    if isinstance(token_usage, Mapping):
        return token_usage
    usage_payload = token_usage_payload_from_boundary_usage(output.get("usage"))
    return usage_payload if isinstance(usage_payload, Mapping) else None


def _outcome_status_from_boundary(value: Any) -> OutcomeStatus:
    return OutcomeStatus(str(value or "").strip().lower())


def _failure_kind_from_boundary(value: Any) -> FailureKind | None:
    if value is None or str(value).strip() == "":
        return None
    return FailureKind(str(value).strip().lower())


def _outcome_from_boundary_payload(payload: Any) -> Outcome:
    if isinstance(payload, Outcome):
        return payload
    if not isinstance(payload, Mapping):
        raise ValueError("Rust boundary outcome response must be an object.")
    status_value = payload.get("status", payload.get("outcome"))
    suggested_next_ids = payload.get("suggested_next_ids", payload.get("suggestedNextIds", []))
    context_updates = payload.get("context_updates", payload.get("contextUpdates", {}))
    retryable = payload.get("retryable")
    return Outcome(
        status=_outcome_status_from_boundary(status_value),
        preferred_label=str(payload.get("preferred_label", payload.get("preferredLabel", "")) or ""),
        suggested_next_ids=[str(item) for item in suggested_next_ids] if isinstance(suggested_next_ids, list) else [],
        context_updates=dict(context_updates) if isinstance(context_updates, Mapping) else {},
        failure_reason=str(payload.get("failure_reason", payload.get("failureReason", "")) or ""),
        notes=str(payload.get("notes") or ""),
        retryable=retryable if isinstance(retryable, bool) else None,
        failure_kind=_failure_kind_from_boundary(payload.get("failure_kind", payload.get("failureKind"))),
        raw_response_text=str(payload.get("raw_response_text", payload.get("rawResponseText", "")) or ""),
    )


def _boundary_output_error_outcome(payload: Any) -> Outcome | None:
    if not isinstance(payload, Mapping):
        return None
    message = str(payload.get("message") or payload.get("error") or "Rust codergen boundary failed.")
    retryable = payload.get("retryable")
    return Outcome(
        status=OutcomeStatus.FAIL,
        failure_reason=message,
        retryable=retryable if isinstance(retryable, bool) else False,
        failure_kind=FailureKind.RUNTIME,
        raw_response_text=json.dumps(dict(payload), sort_keys=True),
    )


def _boundary_response_kind_and_value(output: Mapping[str, Any]) -> tuple[str, Any]:
    response = output.get("response")
    if isinstance(response, Mapping):
        kind = str(response.get("kind") or response.get("type") or "").strip().lower()
        if not kind and ("status" in response or "outcome" in response):
            return "outcome", response
        return kind, response.get("value")
    if isinstance(response, bool):
        return "boolean", response
    if isinstance(response, str):
        return "text", response
    for key in ("final_assistant_text", "assistant_message", "text"):
        if output.get(key) is not None:
            return "text", output.get(key)
    if isinstance(output.get("outcome"), Mapping):
        return "outcome", output.get("outcome")
    return "", None


class UnifiedAgentBackend(CodergenBackend):
    def __init__(
        self,
        working_dir: str,
        emit,
        *,
        provider: str,
        model: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
        client_factory: Any | None = None,
        config_dir: Path | str | None = None,
        boundary: RustAgentBoundary | None = None,
        raw_rpc_logger: Callable[[str, str], None] | None = None,
    ):
        del client_factory
        self.requested_working_dir = normalize_project_path_value(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.provider = _normalize_provider(provider)
        self.model = as_non_empty_string(model)
        self.reasoning_effort = as_non_empty_string(reasoning_effort)
        self.config_dir = Path(config_dir) if config_dir is not None else None
        self.metadata: dict[str, Any] = {}
        if self.config_dir is not None:
            self.metadata["spark.config_dir"] = str(self.config_dir)
        self._on_usage_update = on_usage_update
        self._boundary = boundary or SerializedRustAgentBoundary()
        self._raw_rpc_logger = raw_rpc_logger
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()
        self._active_turn_lock = threading.Lock()
        self._active_turn_id: str | None = None
        self._active_turn_request: dict[str, Any] | None = None

    def _log(self, node_id: str, message: str) -> None:
        if message:
            self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

    def _runtime_failure(self, reason: str, *, retryable: bool | None = None) -> Outcome:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=reason,
            retryable=retryable if retryable is not None else False if _is_provider_setup_failure(reason) else None,
            failure_kind=FailureKind.RUNTIME,
        )

    def _record_usage_delta(self, *, model: Optional[str], delta: TokenUsageBucket) -> None:
        if not delta.has_any_usage():
            return
        normalized_model = str(model or "").strip() or "rust-boundary default"
        with self._token_usage_lock:
            self._token_usage_breakdown.add_for_model(normalized_model, delta)
            snapshot = self._token_usage_breakdown.copy()
        if self._on_usage_update is not None:
            self._on_usage_update(snapshot)

    def _record_boundary_output_usage(self, *, model: Optional[str], output: Mapping[str, Any]) -> None:
        breakdown = TokenUsageBreakdown.from_dict(
            output.get("token_usage_breakdown") if isinstance(output.get("token_usage_breakdown"), Mapping) else None
        )
        if breakdown is not None:
            for model_id, usage in breakdown.by_model.items():
                self._record_usage_delta(model=model_id, delta=usage)
            if not breakdown.by_model:
                self._record_usage_delta(
                    model=model,
                    delta=TokenUsageBucket(
                        input_tokens=breakdown.input_tokens,
                        cached_input_tokens=breakdown.cached_input_tokens,
                        output_tokens=breakdown.output_tokens,
                        total_tokens=breakdown.total_tokens,
                    ),
                )
            return
        token_usage = _boundary_token_usage_payload(output)
        if token_usage is not None:
            self._record_usage_delta(model=model, delta=_usage_bucket_from_boundary_payload(token_usage))
            return
        self._record_usage_delta(model=model, delta=_usage_bucket_from_boundary_payload(output.get("usage")))

    def _emit_raw_log_lines(self, raw_log_lines: Any) -> None:
        if self._raw_rpc_logger is None or not isinstance(raw_log_lines, list):
            return
        for entry in raw_log_lines:
            if isinstance(entry, str):
                self._raw_rpc_logger("incoming", entry)
                continue
            if not isinstance(entry, Mapping):
                continue
            direction = str(entry.get("direction") or "incoming")
            line = str(entry.get("line") or "")
            if line:
                self._raw_rpc_logger(direction, line)

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        with self._active_turn_lock:
            active_turn_id = self._active_turn_id
            active_turn_request = dict(self._active_turn_request or {})
        if not active_turn_id:
            return _rejected_intervention(
                request,
                reason="no_active_turn",
                message="No active Rust boundary codergen turn is available for intervention.",
            )
        steer = getattr(self._boundary, "steer_codergen_turn", None)
        if not callable(steer):
            return _rejected_intervention(
                request,
                reason="backend_steering_unsupported",
                delivery_mode="rust_boundary",
                message="Rust boundary codergen backend does not expose active turn steering through the Python facade.",
            )
        steer_payload = self._build_steer_payload(
            active_turn_id,
            active_turn_request,
            request,
        )
        try:
            output = steer(steer_payload)
        except RustBoundaryError as exc:
            return _rejected_intervention(
                request,
                reason="boundary_steer_failed",
                delivery_mode="rust_boundary",
                message=str(exc),
            )
        except RuntimeError as exc:
            return _rejected_intervention(
                request,
                reason="boundary_steer_failed",
                delivery_mode="rust_boundary",
                message=str(exc),
            )
        if not isinstance(output, Mapping):
            return _rejected_intervention(
                request,
                reason="boundary_steer_invalid_response",
                delivery_mode="rust_boundary",
                message="Rust boundary codergen steering returned a non-object response.",
            )
        status = str(output.get("status") or "").strip().lower()
        delivery_mode = str(output.get("delivery_mode") or output.get("deliveryMode") or "rust_boundary")
        message = str(output.get("message") or "")
        if status in {"delivered", "accepted", "ok", "success"}:
            return _delivered_intervention(
                request,
                delivery_mode=delivery_mode,
                message=message or "Intervention delivered to active Rust boundary codergen turn.",
            )
        return _rejected_intervention(
            request,
            reason=str(output.get("reason") or "boundary_steer_rejected"),
            delivery_mode=delivery_mode,
            message=message,
        )

    def _build_steer_payload(
        self,
        turn_id: str,
        active_turn_request: Mapping[str, Any],
        request: ChildInterventionRequest,
    ) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "turn_id": turn_id,
            "node_id": str(active_turn_request.get("node_id") or request.target_node_id or ""),
            "message": request.message,
            "reason": request.reason,
            "source": request.source,
            "cycle": request.cycle,
            "child_run_id": request.child_run_id,
            "parent_run_id": request.parent_run_id,
            "parent_node_id": request.parent_node_id,
            "root_run_id": request.root_run_id,
            "target_node_id": request.target_node_id,
        }
        for key in (
            "provider",
            "model",
            "llm_profile",
            "reasoning_effort",
            "project_path",
            "metadata",
        ):
            if key in active_turn_request:
                payload[key] = active_turn_request[key]
        return payload

    def _handle_turn_stream_event(
        self,
        node_id: str,
        event_payload: Any,
        emit_event: Optional[Callable[..., None]],
        *,
        model: Optional[str],
        previous_total: TokenUsageBucket | None,
    ) -> tuple[bool, TokenUsageBucket | None]:
        event = turn_stream_event_from_boundary_payload(event_payload)
        _emit_turn_stream_progress(emit_event, node_id=node_id, event=event)
        if event.kind == "token_usage_updated" and event.token_usage is not None:
            return self._record_boundary_usage_event(
                model=model,
                token_usage=event.token_usage,
                previous_total=previous_total,
            )
        if event.kind == "content_delta" and event.channel == "assistant" and event.content_delta:
            self._log(node_id, event.content_delta)
        if event.tool_call is not None:
            title = str(getattr(event.tool_call, "title", "") or getattr(event.tool_call, "kind", "") or "tool")
            status = str(getattr(event.tool_call, "status", "") or "")
            if title:
                self._log(node_id, f"tool {status or 'event'}: {title}")
        if event.error:
            self._log(node_id, event.error)
        return False, previous_total

    def _record_boundary_usage_event(
        self,
        *,
        model: Optional[str],
        token_usage: Mapping[str, Any],
        previous_total: TokenUsageBucket | None,
    ) -> tuple[bool, TokenUsageBucket | None]:
        if isinstance(token_usage.get("total"), Mapping) or isinstance(token_usage.get("last"), Mapping):
            usage_payload: Mapping[str, Any] | None = token_usage
        else:
            usage_payload = token_usage_payload_from_boundary_usage(token_usage)
        delta, next_total = compute_live_usage_delta(usage_payload, previous_total)
        if delta is None:
            return False, next_total
        if delta.has_any_usage():
            self._record_usage_delta(model=model, delta=delta)
        return True, next_total

    def _handle_boundary_event(
        self,
        node_id: str,
        event_payload: Any,
        emit_event: Optional[Callable[..., None]],
        *,
        model: Optional[str],
        previous_total: TokenUsageBucket | None,
    ) -> tuple[bool, TokenUsageBucket | None]:
        if not isinstance(event_payload, Mapping):
            return False, previous_total
        if "kind" in event_payload:
            return self._handle_turn_stream_event(
                node_id,
                event_payload,
                emit_event,
                model=model,
                previous_total=previous_total,
            )
        event_type = str(event_payload.get("event_type") or event_payload.get("type") or "").strip()
        payload = event_payload.get("payload")
        if isinstance(payload, Mapping):
            if "kind" in payload:
                return self._handle_turn_stream_event(
                    node_id,
                    payload,
                    emit_event,
                    model=model,
                    previous_total=previous_total,
                )
            nested_event = payload.get("event") or payload.get("turn_stream_event")
            if isinstance(nested_event, Mapping):
                return self._handle_turn_stream_event(
                    node_id,
                    nested_event,
                    emit_event,
                    model=model,
                    previous_total=previous_total,
                )
            raw_log_line = payload.get("raw_log_line")
            if isinstance(raw_log_line, Mapping):
                self._emit_raw_log_lines([raw_log_line])
            token_usage = payload.get("token_usage") or payload.get("usage")
            if isinstance(token_usage, Mapping):
                return self._record_boundary_usage_event(
                    model=model,
                    token_usage=token_usage,
                    previous_total=previous_total,
                )
            message = payload.get("message") or payload.get("text") or payload.get("content") or payload.get("error")
            if message is not None:
                self._log(node_id, str(message))
        elif event_type:
            self._log(node_id, event_type)
        return False, previous_total

    def _handle_boundary_events(
        self,
        node_id: str,
        output: Mapping[str, Any],
        emit_event: Optional[Callable[..., None]],
        *,
        model: Optional[str],
    ) -> bool:
        saw_usage = False
        events = output.get("events")
        if not isinstance(events, list):
            return False
        previous_total: TokenUsageBucket | None = None
        for event_payload in events:
            event_saw_usage, previous_total = self._handle_boundary_event(
                node_id,
                event_payload,
                emit_event,
                model=model,
                previous_total=previous_total,
            )
            saw_usage = event_saw_usage or saw_usage
        return saw_usage

    def _run_boundary_once(
        self,
        request: dict[str, Any],
        *,
        node_id: str,
        model: Optional[str],
        emit_event: Optional[Callable[..., None]],
    ) -> dict[str, Any]:
        request = dict(request)
        request["turn_id"] = as_non_empty_string(request.get("turn_id")) or f"codergen-{uuid.uuid4().hex}"
        with self._active_turn_lock:
            self._active_turn_id = request["turn_id"]
            self._active_turn_request = dict(request)
        try:
            output = self._run_boundary_call(request)
        finally:
            with self._active_turn_lock:
                if self._active_turn_id == request["turn_id"]:
                    self._active_turn_id = None
                    self._active_turn_request = None
        if not isinstance(output, Mapping):
            raise RuntimeError("Rust codergen boundary returned a non-object output.")
        output = dict(output)
        self._emit_raw_log_lines(output.get("raw_log_lines"))
        saw_usage_event = self._handle_boundary_events(node_id, output, emit_event, model=model)
        if not saw_usage_event:
            self._record_boundary_output_usage(model=model, output=output)
        return output

    def _run_boundary_call(self, request: dict[str, Any]) -> dict[str, Any]:
        timeout = request.get("timeout_seconds")
        if timeout is None:
            return self._boundary.run_codergen(request)
        try:
            timeout_seconds = max(0.0, float(timeout))
        except (TypeError, ValueError):
            return self._boundary.run_codergen(request)

        result_queue: queue.Queue[tuple[bool, Any]] = queue.Queue(maxsize=1)

        def run() -> None:
            try:
                result_queue.put((True, self._boundary.run_codergen(request)))
            except BaseException as exc:  # noqa: BLE001
                result_queue.put((False, exc))

        thread = threading.Thread(target=run, name="rust-codergen-boundary", daemon=True)
        thread.start()
        try:
            succeeded, value = result_queue.get(timeout=timeout_seconds)
        except queue.Empty as exc:
            raise RuntimeError(f"Rust codergen boundary timed out after {timeout_seconds:g}s") from exc
        if succeeded:
            return value
        if isinstance(value, BaseException):
            raise value
        raise RuntimeError("Rust codergen boundary failed without an exception payload.")

    def _coerce_boundary_output(
        self,
        output: Mapping[str, Any],
        *,
        response_contract: str,
        write_contract: ContextWriteContract | None,
    ) -> str | Outcome | _StructuredContractViolation:
        output_error = _boundary_output_error_outcome(output.get("error"))
        if output_error is not None:
            return output_error
        kind, value = _boundary_response_kind_and_value(output)
        if kind in {"text", "plain_text", ""}:
            response_text = str(value or "")
            result = _coerce_structured_text_outcome(response_text, response_contract=response_contract)
            if isinstance(result, Outcome):
                return result
            if isinstance(result, _ModeledOutcomeParseResult):
                violation = _validate_write_contract_violation(
                    result.outcome,
                    write_contract=write_contract,
                    response_contract=response_contract,
                    raw_text=response_text,
                )
                if violation is None:
                    return result.outcome
                return violation
            if isinstance(result, _PlainTextParseResult):
                return result.raw_text
            return _with_write_contract(result, write_contract)
        if kind in {"boolean", "bool"}:
            succeeded = bool(value)
            return Outcome(
                status=OutcomeStatus.SUCCESS if succeeded else OutcomeStatus.FAIL,
                notes="codergen backend success" if succeeded else "",
                failure_reason="" if succeeded else "codergen backend failure",
                failure_kind=FailureKind.RUNTIME if not succeeded else None,
            )
        if kind == "outcome":
            outcome = _outcome_from_boundary_payload(value)
            response_text = outcome.raw_response_text or json.dumps(outcome.to_payload(), sort_keys=True)
            violation = _validate_write_contract_violation(
                outcome,
                write_contract=write_contract,
                response_contract=response_contract,
                raw_text=response_text,
            )
            if violation is None:
                return outcome
            return violation
        return self._runtime_failure(f"Rust codergen boundary returned unsupported response kind: {kind or '<missing>'}")

    def _coerce_or_repair_contract_result(
        self,
        *,
        base_request: dict[str, Any],
        initial_output: Mapping[str, Any],
        node_id: str,
        response_contract: str,
        contract_repair_attempts: int,
        write_contract: ContextWriteContract | None,
        model: Optional[str],
        emit_event: Optional[Callable[..., None]],
    ) -> str | Outcome:
        result = self._coerce_boundary_output(
            initial_output,
            response_contract=response_contract,
            write_contract=write_contract,
        )
        if not isinstance(result, _StructuredContractViolation):
            return result
        if contract_repair_attempts <= 0:
            return _contract_failure_outcome(result)
        current_violation = _with_write_contract(result, write_contract)
        for attempt in range(1, contract_repair_attempts + 1):
            self._log(
                node_id,
                f"response contract violation for {node_id}; requesting corrected final answer "
                f"(attempt {attempt}/{contract_repair_attempts}): {current_violation.reason}",
            )
            repair_request = dict(base_request)
            repair_request["prompt"] = _build_contract_repair_prompt(current_violation)
            repair_request["repair_attempt"] = attempt
            repair_output = self._run_boundary_once(
                repair_request,
                node_id=node_id,
                model=model,
                emit_event=emit_event,
            )
            repaired = self._coerce_boundary_output(
                repair_output,
                response_contract=current_violation.response_contract,
                write_contract=write_contract,
            )
            if isinstance(repaired, _StructuredContractViolation):
                current_violation = _with_write_contract(repaired, write_contract)
                continue
            return repaired
        return _contract_failure_outcome(current_violation)

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: Optional[float] = None,
        model: Optional[str] = None,
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        llm_profile: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        try:
            effective_provider = _normalize_provider(provider or self.provider)
            if not llm_profile and effective_provider not in UNIFIED_AGENT_PROVIDERS:
                return self._runtime_failure(
                    f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}",
                    retryable=False,
                )
            effective_model = as_non_empty_string(model) or self.model
            effective_reasoning_effort = as_non_empty_string(reasoning_effort) or self.reasoning_effort
            request = build_codergen_backend_request_payload(
                node_id=node_id,
                prompt=prompt,
                context=_context_snapshot(context),
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                timeout=timeout,
                write_contract=write_contract,
                provider=effective_provider,
                model=effective_model,
                llm_profile=llm_profile,
                reasoning_effort=effective_reasoning_effort,
                project_path=self.requested_working_dir,
                metadata=self.metadata,
            )
            output = self._run_boundary_once(
                request,
                node_id=node_id,
                model=effective_model,
                emit_event=emit_event,
            )
            result = self._coerce_or_repair_contract_result(
                base_request=request,
                initial_output=output,
                node_id=node_id,
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                write_contract=write_contract,
                model=effective_model,
                emit_event=emit_event,
            )
            if isinstance(result, str) and result:
                self._log(node_id, result)
            elif isinstance(result, Outcome) and result.failure_reason:
                self._log(node_id, result.failure_reason)
            return result
        except RustBoundaryError as exc:
            return self._runtime_failure(str(exc), retryable=exc.retryable)
        except RuntimeError as exc:
            return self._runtime_failure(str(exc))
        except (ValueError, TypeError) as exc:
            return self._runtime_failure(str(exc) or exc.__class__.__name__)


class ProviderRouterBackend(CodergenBackend):
    def __init__(
        self,
        working_dir: str,
        emit,
        *,
        model: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
        config_dir: Path | str | None = None,
        boundary: RustAgentBoundary | None = None,
    ):
        self.working_dir = working_dir
        self.emit = emit
        self.model = model
        self.provider = "codex"
        self.config_dir = Path(config_dir) if config_dir is not None else None
        self._boundary = boundary
        self._on_usage_update = on_usage_update
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()
        self._source_usage_snapshots: dict[object, TokenUsageBreakdown] = {}
        self._active_backend_lock = threading.Lock()
        self._active_backend: object | None = None
        self._raw_rpc_log_lock = threading.Lock()
        self._raw_rpc_log_state = threading.local()
        self._codex_usage_source = object()
        self._codex = CodexAppServerBackend(
            working_dir,
            emit,
            model=model,
            on_usage_update=lambda snapshot: self._record_source_usage(
                self._codex_usage_source,
                snapshot,
            ),
        )

    def _stage_raw_rpc_log_path(self, node_id: str, logs_root: str | Path | None) -> Path | None:
        if logs_root is None:
            return None
        stage_dir = Path(logs_root) / node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        return stage_dir / "raw-rpc.jsonl"

    def _append_raw_rpc_log(self, direction: str, line: str) -> None:
        path = getattr(self._raw_rpc_log_state, "path", None)
        if path is None:
            return
        payload = {
            "timestamp": datetime.now(UTC).isoformat(),
            "direction": direction,
            "line": line,
        }
        with self._raw_rpc_log_lock:
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(payload, sort_keys=True) + "\n")

    @contextmanager
    def bind_stage_raw_rpc_log(self, node_id: str, logs_root: str | Path | None):
        previous = getattr(self._raw_rpc_log_state, "path", None)
        self._raw_rpc_log_state.path = self._stage_raw_rpc_log_path(node_id, logs_root)
        try:
            with self._codex.bind_stage_raw_rpc_log(node_id, logs_root):
                yield
        finally:
            if previous is None:
                if hasattr(self._raw_rpc_log_state, "path"):
                    delattr(self._raw_rpc_log_state, "path")
            else:
                self._raw_rpc_log_state.path = previous

    def _record_source_usage(self, source_key: object, source_snapshot: TokenUsageBreakdown) -> None:
        with self._token_usage_lock:
            previous_snapshot = self._source_usage_snapshots.get(source_key)
            source_delta = _breakdown_delta_from(source_snapshot, previous_snapshot)
            self._source_usage_snapshots[source_key] = source_snapshot.copy()
            if not source_delta.has_any_usage():
                return
            for model_id, usage in source_delta.by_model.items():
                self._token_usage_breakdown.add_for_model(model_id, usage)
            snapshot = self._token_usage_breakdown.copy()
        if self._on_usage_update is not None:
            self._on_usage_update(snapshot)

    def _set_active_backend(self, backend: object | None) -> None:
        with self._active_backend_lock:
            self._active_backend = backend

    def _clear_active_backend(self, backend: object) -> None:
        with self._active_backend_lock:
            if self._active_backend is backend:
                self._active_backend = None

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        with self._active_backend_lock:
            backend = self._active_backend
        if backend is None:
            return _rejected_intervention(
                request,
                reason="no_active_turn",
                message="No active provider backend is available for intervention.",
            )
        requester = getattr(backend, "request_child_intervention", None)
        if not callable(requester):
            return _rejected_intervention(
                request,
                reason="backend_steering_unsupported",
                delivery_mode="unsupported",
                message="Active provider backend does not support intervention.",
            )
        return requester(request)

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: Optional[float] = None,
        model: Optional[str] = None,
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        llm_profile: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        try:
            effective_provider = _normalize_provider(provider)
        except ValueError as exc:
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=str(exc),
                retryable=False,
                failure_kind=FailureKind.RUNTIME,
            )
        if effective_provider == "codex" and not llm_profile:
            self._set_active_backend(self._codex)
            try:
                return self._codex.run(
                    node_id,
                    prompt,
                    context,
                    response_contract=response_contract,
                    contract_repair_attempts=contract_repair_attempts,
                    timeout=timeout,
                    model=model,
                    reasoning_effort=reasoning_effort,
                    emit_event=emit_event,
                    write_contract=write_contract,
                )
            finally:
                self._clear_active_backend(self._codex)
        if llm_profile or effective_provider in UNIFIED_AGENT_PROVIDERS:
            usage_source = object()
            backend = UnifiedAgentBackend(
                self.working_dir,
                self.emit,
                provider=effective_provider,
                model=model or self.model,
                reasoning_effort=reasoning_effort,
                on_usage_update=lambda snapshot: self._record_source_usage(usage_source, snapshot),
                config_dir=self.config_dir,
                boundary=self._boundary,
                raw_rpc_logger=self._append_raw_rpc_log,
            )
            kwargs = {
                "response_contract": response_contract,
                "contract_repair_attempts": contract_repair_attempts,
                "timeout": timeout,
                "model": model,
                "provider": effective_provider,
                "reasoning_effort": reasoning_effort,
                "emit_event": emit_event,
                "write_contract": write_contract,
            }
            if llm_profile:
                kwargs["llm_profile"] = llm_profile
            self._set_active_backend(backend)
            try:
                return backend.run(node_id, prompt, context, **kwargs)
            finally:
                self._clear_active_backend(backend)
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=(
                f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}"
            ),
            retryable=False,
            failure_kind=FailureKind.RUNTIME,
        )


def build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
    on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
    config_dir: Path | str | None = None,
    boundary: RustAgentBoundary | None = None,
) -> CodergenBackend:
    normalized = backend_name.strip().lower()
    if normalized in {"", "provider-router"}:
        return ProviderRouterBackend(
            working_dir,
            emit,
            model=model,
            on_usage_update=on_usage_update,
            config_dir=config_dir,
            boundary=boundary,
        )
    if normalized == "codex-app-server":
        return CodexAppServerBackend(working_dir, emit, model=model, on_usage_update=on_usage_update)
    raise ValueError(
        "Unsupported backend. Supported backends: provider-router, codex-app-server."
    )
