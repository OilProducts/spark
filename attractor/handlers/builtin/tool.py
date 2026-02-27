from __future__ import annotations

import json
import os
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

        command = str(cmd_attr.value)
        hook_metadata = {
            "node_id": runtime.node_id,
            "tool_command": command,
        }

        pre_hook = _resolve_hook_command(runtime, "tool_hooks.pre")
        if pre_hook:
            pre_hook_result = _run_hook(pre_hook, hook_phase="pre", metadata=hook_metadata)
            _record_hook_failure(runtime, command=pre_hook, hook_phase="pre", result=pre_hook_result)
        post_hook = _resolve_hook_command(runtime, "tool_hooks.post")

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
        finally:
            if post_hook:
                post_hook_result = _run_hook(post_hook, hook_phase="post", metadata=hook_metadata)
                _record_hook_failure(runtime, command=post_hook, hook_phase="post", result=post_hook_result)


def _resolve_hook_command(runtime: HandlerRuntime, key: str) -> str:
    node_attr = runtime.node_attrs.get(key)
    if node_attr and str(node_attr.value).strip():
        return str(node_attr.value).strip()

    graph_attr = runtime.graph.graph_attrs.get(key)
    if graph_attr and str(graph_attr.value).strip():
        return str(graph_attr.value).strip()

    return ""


def _run_hook(command: str, *, hook_phase: str, metadata: dict[str, str]) -> subprocess.CompletedProcess[str]:
    payload = {
        "hook_phase": hook_phase,
        "node_id": metadata.get("node_id", ""),
        "tool_command": metadata.get("tool_command", ""),
    }
    env = os.environ.copy()
    env.update(
        {
            "ATTRACTOR_TOOL_HOOK_PHASE": payload["hook_phase"],
            "ATTRACTOR_TOOL_NODE_ID": payload["node_id"],
            "ATTRACTOR_TOOL_COMMAND": payload["tool_command"],
        }
    )
    try:
        return subprocess.run(
            command,
            shell=True,
            capture_output=True,
            text=True,
            input=json.dumps(payload),
            env=env,
        )
    except Exception as exc:
        reason = str(exc) or exc.__class__.__name__
        return subprocess.CompletedProcess(command, -1, stdout="", stderr=reason)


def _record_hook_failure(
    runtime: HandlerRuntime,
    *,
    command: str,
    hook_phase: str,
    result: subprocess.CompletedProcess[str],
) -> None:
    if result.returncode == 0:
        return
    if not runtime.logs_root:
        return
    try:
        stage_dir = runtime.logs_root / runtime.node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        record = {
            "hook_phase": hook_phase,
            "command": command,
            "exit_code": int(result.returncode),
            "stdout": str(result.stdout or "").strip(),
            "stderr": str(result.stderr or "").strip(),
        }
        with (stage_dir / "tool_hook_failures.jsonl").open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")
    except OSError:
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
