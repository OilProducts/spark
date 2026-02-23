from __future__ import annotations

import subprocess
from typing import Any

from attractor.dsl.models import Duration
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ToolHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        cmd_attr = runtime.node_attrs.get("tool_command")
        if not cmd_attr or not str(cmd_attr.value).strip():
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No tool_command specified")

        command = str(cmd_attr.value)
        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        try:
            proc = subprocess.run(command, shell=True, capture_output=True, text=True, timeout=timeout)
            if proc.returncode == 0:
                notes = proc.stdout.strip()
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    notes=notes,
                    context_updates={
                        "tool.output": proc.stdout.strip(),
                        "tool.exit_code": proc.returncode,
                    },
                )

            reason = proc.stderr.strip() or f"tool command failed with code {proc.returncode}"
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "tool.output": proc.stdout.strip(),
                    "tool.exit_code": proc.returncode,
                },
            )
        except subprocess.TimeoutExpired:
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason="tool command timed out",
                context_updates={
                    "tool.output": "",
                    "tool.exit_code": -1,
                },
            )


def _to_seconds(attr: Any) -> float | None:
    if not attr:
        return None
    value = attr.value
    if isinstance(value, Duration):
        unit = value.unit
        if unit == "ms":
            return value.value / 1000
        if unit == "s":
            return value.value
        if unit == "m":
            return value.value * 60
        if unit == "h":
            return value.value * 3600
        if unit == "d":
            return value.value * 86400
    if isinstance(value, (int, float)):
        return float(value)
    try:
        return float(str(value))
    except (TypeError, ValueError):
        return None
