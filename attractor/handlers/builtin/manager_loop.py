from __future__ import annotations

import re
import time

from attractor.dsl.models import DotAttribute, Duration
from attractor.engine.conditions import evaluate_condition
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ManagerLoopHandler:
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        poll_interval = _poll_interval_seconds(runtime.node_attrs.get("manager.poll_interval"))
        max_cycles = _max_cycles(runtime.node_attrs.get("manager.max_cycles"))
        stop_condition = _stop_condition(runtime.node_attrs.get("manager.stop_condition"))
        actions = _manager_actions(runtime.node_attrs.get("manager.actions"))
        steer_cooldown = _steer_cooldown_seconds(runtime.node_attrs.get("manager.steer_cooldown"))
        last_steer_at: float | None = None

        for cycle in range(1, max_cycles + 1):
            if "observe" in actions:
                _ingest_child_telemetry(runtime.context, runtime.node_id, cycle)

            now = time.monotonic()
            if "steer" in actions and _steer_cooldown_elapsed(
                now,
                last_steer_at=last_steer_at,
                cooldown_seconds=steer_cooldown,
            ):
                _steer_child(runtime.context, runtime.node_id, cycle)
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
    if child_outcome == OutcomeStatus.SUCCESS.value:
        return Outcome(status=OutcomeStatus.SUCCESS, notes="Child completed")
    if child_status == "failed":
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="Child failed")
    return None


def _ingest_child_telemetry(context: Context, node_id: str, cycle: int) -> None:
    del context, node_id, cycle


def _steer_child(context: Context, node_id: str, cycle: int) -> None:
    del context, node_id, cycle


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
