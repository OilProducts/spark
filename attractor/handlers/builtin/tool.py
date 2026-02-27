from __future__ import annotations

import subprocess
from typing import Any

from attractor.dsl.models import Duration
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ToolHandler:
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        cmd_attr = runtime.node_attrs.get("tool_command")
        if not cmd_attr or not str(cmd_attr.value).strip():
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No tool_command specified")

        pre_hook = _resolve_hook_command(runtime, "tool_hooks.pre")
        if pre_hook:
            _run_hook(pre_hook)

        command = str(cmd_attr.value)
        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        try:
            proc = subprocess.run(command, shell=True, capture_output=True, text=True, timeout=timeout)
            _write_output_artifact(runtime, proc.stdout)
            if proc.returncode == 0:
                notes = proc.stdout.strip()
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    notes=notes,
                    context_updates={
                        "context.tool.output": proc.stdout.strip(),
                        "context.tool.exit_code": proc.returncode,
                    },
                )

            reason = proc.stderr.strip() or f"tool command failed with code {proc.returncode}"
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "context.tool.output": proc.stdout.strip(),
                    "context.tool.exit_code": proc.returncode,
                },
            )
        except subprocess.TimeoutExpired as exc:
            timeout_output = str(exc.stdout or "")
            _write_output_artifact(runtime, timeout_output)
            reason = str(exc) or "tool command timed out"
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "context.tool.output": timeout_output.strip(),
                    "context.tool.exit_code": -1,
                },
            )
        except Exception as exc:
            _write_output_artifact(runtime, "")
            reason = str(exc) or "tool command execution error"
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "context.tool.output": "",
                    "context.tool.exit_code": -1,
                },
            )


def _resolve_hook_command(runtime: HandlerRuntime, key: str) -> str:
    node_attr = runtime.node_attrs.get(key)
    if node_attr and str(node_attr.value).strip():
        return str(node_attr.value).strip()

    graph_attr = runtime.graph.graph_attrs.get(key)
    if graph_attr and str(graph_attr.value).strip():
        return str(graph_attr.value).strip()

    return ""


def _run_hook(command: str) -> None:
    try:
        subprocess.run(command, shell=True, capture_output=True, text=True)
    except Exception:
        return


def _write_output_artifact(runtime: HandlerRuntime, output: str) -> None:
    if not runtime.logs_root:
        return
    try:
        stage_dir = runtime.logs_root / runtime.node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        (stage_dir / "tool_output.txt").write_text(output, encoding="utf-8")
    except OSError:
        return


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
