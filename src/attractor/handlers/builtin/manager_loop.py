from __future__ import annotations

import json
import os
from pathlib import Path
import re
import time
from typing import Any

from attractor.dsl import DiagnosticSeverity, DotParseError, parse_dot
from attractor.dsl.models import DotAttribute, Duration
from attractor.engine.conditions import evaluate_condition
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.graph_prep import prepare_graph
from attractor.handlers.runner import HandlerRunner

from ..base import HandlerRuntime


class ManagerLoopHandler:
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        startup_error = _autostart_child_pipeline(runtime)
        if startup_error is not None:
            return startup_error

        poll_interval = _poll_interval_seconds(runtime.node_attrs.get("manager.poll_interval"))
        max_cycles = _max_cycles(runtime.node_attrs.get("manager.max_cycles"))
        stop_condition = _stop_condition(runtime.node_attrs.get("manager.stop_condition"))
        actions = _manager_actions(runtime.node_attrs.get("manager.actions"))
        steer_cooldown = _steer_cooldown_seconds(runtime.node_attrs.get("manager.steer_cooldown"))
        last_steer_at: float | None = None

        for cycle in range(1, max_cycles + 1):
            if "observe" in actions:
                _ingest_child_telemetry(runtime.context, runtime.node_id, cycle)
                _append_manager_artifact(
                    runtime.logs_root,
                    runtime.node_id,
                    "manager_telemetry.jsonl",
                    _telemetry_payload(runtime.context, runtime.node_id, cycle),
                )

            now = time.monotonic()
            if "steer" in actions and _steer_cooldown_elapsed(
                now,
                last_steer_at=last_steer_at,
                cooldown_seconds=steer_cooldown,
            ):
                _steer_child(runtime.context, runtime.node_id, cycle)
                _append_manager_artifact(
                    runtime.logs_root,
                    runtime.node_id,
                    "manager_interventions.jsonl",
                    _intervention_payload(runtime.context, runtime.node_id, cycle),
                )
                last_steer_at = now

            child_resolution = _resolve_child_status(runtime.context)
            if child_resolution is not None:
                return child_resolution

            if stop_condition and _stop_condition_met(stop_condition, runtime.context):
                return Outcome(status=OutcomeStatus.SUCCESS, notes="Stop condition satisfied")

            if "wait" in actions and poll_interval > 0:
                time.sleep(poll_interval)

        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason="Max cycles exceeded",
        )


def _autostart_child_pipeline(runtime: HandlerRuntime) -> Outcome | None:
    child_dotfile = _dot_attr_string(runtime.graph.graph_attrs.get("stack.child_dotfile"))
    if not child_dotfile:
        return None
    if not _child_autostart_enabled(runtime.node_attrs.get("stack.child_autostart")):
        return None

    current_status = str(runtime.context.get("context.stack.child.status", "")).strip().lower()
    if current_status in {"running", "completed", "failed"}:
        return None

    authored_child_workdir = _authored_dot_attr_string(runtime.graph.graph_attrs.get("stack.child_workdir"))
    child_workdir_path = _resolve_child_workdir_path(
        runtime.context,
        authored_child_workdir=authored_child_workdir,
    )
    child_dot_path = _resolve_child_dot_path(
        child_dotfile,
        child_workdir_path=child_workdir_path,
        context=runtime.context,
        child_workdir_is_authored=bool(authored_child_workdir),
    )
    if not child_dot_path.exists():
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"Child DOT file not found: {child_dot_path}",
        )

    try:
        child_graph = parse_dot(child_dot_path.read_text(encoding="utf-8"))
    except OSError as exc:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"Unable to read child DOT file: {exc}",
        )
    except DotParseError as exc:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"Failed to parse child DOT graph: {exc}",
        )
    child_graph, diagnostics = prepare_graph(child_graph)
    child_errors = [diag for diag in diagnostics if diag.severity == DiagnosticSeverity.ERROR]
    if child_errors:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"Child DOT graph failed validation: {child_errors[0].message}",
        )

    registry = getattr(runtime.runner, "registry", None)
    if registry is None:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason="Manager loop requires HandlerRunner-compatible runtime.runner",
        )

    child_logs_root = runtime.logs_root / runtime.node_id / "child" if runtime.logs_root else None
    child_runner = HandlerRunner(child_graph, registry, logs_root=child_logs_root)
    child_executor = PipelineExecutor(
        child_graph,
        child_runner,
        logs_root=str(child_logs_root) if child_logs_root else None,
    )
    try:
        child_result = _run_child_executor_in_workdir(
            child_executor,
            context=runtime.context.clone(),
            workdir=child_workdir_path,
        )
    except OSError as exc:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"Unable to run child pipeline from {child_workdir_path}: {exc}",
        )

    child_status = "completed" if child_result.status == "completed" else "failed"
    runtime.context.set("context.stack.child.status", child_status)
    runtime.context.set("context.stack.child.outcome", child_result.outcome or "")
    runtime.context.set("context.stack.child.outcome_reason_code", child_result.outcome_reason_code or "")
    runtime.context.set("context.stack.child.outcome_reason_message", child_result.outcome_reason_message or "")
    runtime.context.set("context.stack.child.active_stage", child_result.current_node)
    runtime.context.set("context.stack.child.completed_nodes", list(child_result.completed_nodes))
    runtime.context.set("context.stack.child.route_trace", list(child_result.route_trace))
    if child_result.failure_reason:
        runtime.context.set("context.stack.child.failure_reason", child_result.failure_reason)

    return None


def _poll_interval_seconds(attr: DotAttribute | None) -> float:
    if attr is None:
        return 45.0

    value = attr.value
    if isinstance(value, Duration):
        return _seconds_from_duration(value.value, value.unit)
    if isinstance(value, bool):
        return float(value)
    if isinstance(value, (int, float)):
        return float(value)
    return _parse_duration_string(str(value), 45.0)


def _max_cycles(attr: DotAttribute | None) -> int:
    if attr is None:
        return 1000

    value = attr.value
    if isinstance(value, bool):
        return 1 if value else 0
    if isinstance(value, int):
        return max(0, value)
    if isinstance(value, float):
        return max(0, int(value))
    try:
        return max(0, int(str(value).strip()))
    except ValueError:
        return 1000


def _child_autostart_enabled(attr: DotAttribute | None) -> bool:
    if attr is None:
        return True

    value = attr.value
    if isinstance(value, bool):
        return value

    normalized = str(value).strip().lower()
    if normalized in {"false", "0", "no", "off"}:
        return False
    if normalized in {"true", "1", "yes", "on"}:
        return True
    return True


def _steer_cooldown_seconds(attr: DotAttribute | None) -> float:
    if attr is None:
        return 0.0
    return _poll_interval_seconds(attr)


def _steer_cooldown_elapsed(now: float, *, last_steer_at: float | None, cooldown_seconds: float) -> bool:
    if cooldown_seconds <= 0 or last_steer_at is None:
        return True
    return now - last_steer_at >= cooldown_seconds


def _manager_actions(attr: DotAttribute | None) -> set[str]:
    if attr is None:
        return {"observe", "wait"}

    raw = str(attr.value).strip()
    if raw == "":
        return set()

    actions = {token.strip().lower() for token in raw.split(",")}
    return {action for action in actions if action in {"observe", "steer", "wait"}}


def _dot_attr_string(attr: DotAttribute | None) -> str:
    if attr is None:
        return ""
    value = attr.value
    if hasattr(value, "raw"):
        return str(value.raw).strip()
    return str(value).strip()


def _authored_dot_attr_string(attr: DotAttribute | None) -> str:
    if attr is None or attr.line == 0:
        return ""
    return _dot_attr_string(attr)


def _context_dir(context: Context, key: str) -> Path | None:
    raw_value = str(context.get(key, "")).strip()
    if not raw_value:
        return None
    return Path(raw_value).expanduser().resolve()


def _resolve_child_workdir_path(context: Context, *, authored_child_workdir: str) -> Path:
    run_workdir = _context_dir(context, "internal.run_workdir")
    if authored_child_workdir:
        base_dir = run_workdir or Path.cwd().resolve()
        return _resolve_path_from_base(authored_child_workdir, base_dir)
    return run_workdir or Path.cwd().resolve()


def _resolve_child_dot_path(
    child_dotfile: str,
    *,
    child_workdir_path: Path,
    context: Context,
    child_workdir_is_authored: bool,
) -> Path:
    child_dot_path = Path(child_dotfile)
    if child_dot_path.is_absolute():
        return child_dot_path.resolve()
    if child_workdir_is_authored:
        base_dir = child_workdir_path
    else:
        base_dir = _context_dir(context, "internal.flow_source_dir") or child_workdir_path
    return _resolve_path_from_base(child_dot_path, base_dir)


def _resolve_path_from_base(raw_path: str | Path, base_dir: Path) -> Path:
    candidate = Path(raw_path)
    if candidate.is_absolute():
        return candidate.resolve()
    return (base_dir / candidate).resolve()


def _stop_condition(attr: DotAttribute | None) -> str:
    if attr is None:
        return ""
    return str(attr.value).strip()


def _stop_condition_met(stop_condition: str, context: Context) -> bool:
    probe = Outcome(status=OutcomeStatus.SUCCESS)
    return evaluate_condition(stop_condition, probe, context)


def _resolve_child_status(context: Context) -> Outcome | None:
    child_status = str(context.get("context.stack.child.status", "")).strip().lower()
    if child_status not in {"completed", "failed"}:
        return None

    child_outcome = str(context.get("context.stack.child.outcome", "")).strip().lower()
    if child_status == "completed" and child_outcome == OutcomeStatus.SUCCESS.value:
        return Outcome(status=OutcomeStatus.SUCCESS, notes="Child completed")
    if child_status == "completed" and child_outcome == "failure":
        failure_reason = str(context.get("context.stack.child.outcome_reason_message", "")).strip()
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=failure_reason or "Child completed with failure outcome",
        )
    if child_status == "failed":
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="Child failed")
    return None


def _ingest_child_telemetry(context: Context, node_id: str, cycle: int) -> None:
    del context, node_id, cycle


def _steer_child(context: Context, node_id: str, cycle: int) -> None:
    del context, node_id, cycle


def _telemetry_payload(context: Context, node_id: str, cycle: int) -> dict[str, Any]:
    return {
        "cycle": cycle,
        "node_id": node_id,
        "timestamp_unix": time.time(),
        "child_status": context.get("context.stack.child.status", ""),
        "child_outcome": context.get("context.stack.child.outcome", ""),
        "child_active_stage": context.get("context.stack.child.active_stage", ""),
        "child_retry_count": context.get("context.stack.child.retry_count", ""),
    }


def _intervention_payload(context: Context, node_id: str, cycle: int) -> dict[str, Any]:
    return {
        "cycle": cycle,
        "node_id": node_id,
        "timestamp_unix": time.time(),
        "child_status": context.get("context.stack.child.status", ""),
        "child_active_stage": context.get("context.stack.child.active_stage", ""),
        "instruction": context.get("context.stack.child.intervention", ""),
    }


def _append_manager_artifact(
    logs_root: Path | None,
    node_id: str,
    filename: str,
    payload: dict[str, Any],
) -> None:
    if not logs_root:
        return
    try:
        stage_dir = logs_root / node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        with (stage_dir / filename).open("a", encoding="utf-8") as handle:
            json.dump(payload, handle, sort_keys=True)
            handle.write("\n")
    except OSError:
        return


def _run_child_executor_in_workdir(
    child_executor: PipelineExecutor,
    *,
    context: Context,
    workdir: Path,
):
    original_cwd = Path.cwd()
    os.chdir(workdir)
    try:
        return child_executor.run(context=context, resume=False)
    finally:
        os.chdir(original_cwd)


def _parse_duration_string(raw: str, default: float) -> float:
    value = raw.strip().lower()
    if not value:
        return default
    try:
        return float(value)
    except ValueError:
        match = re.fullmatch(r"(?P<num>\d+)(?P<unit>ms|s|m|h|d)", value)
        if not match:
            return default
        return _seconds_from_duration(int(match.group("num")), match.group("unit"))


def _seconds_from_duration(amount: int, unit: str) -> float:
    if unit == "ms":
        return amount / 1000
    if unit == "s":
        return float(amount)
    if unit == "m":
        return amount * 60.0
    if unit == "h":
        return amount * 3600.0
    if unit == "d":
        return amount * 86400.0
    return 45.0
