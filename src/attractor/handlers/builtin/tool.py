from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
from typing import Any

from attractor.dsl.models import Duration
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ToolHandler:
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        cmd_attr = runtime.node_attrs.get("tool.command")
        if not cmd_attr or not str(cmd_attr.value).strip():
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No tool.command specified")

        command = str(cmd_attr.value)
        tool_cwd = _resolve_tool_cwd(runtime)
        hook_metadata = {
            "node_id": runtime.node_id,
            "tool_command": command,
        }

        pre_hook = _resolve_hook_command(runtime, "tool.hooks.pre")
        if pre_hook:
            pre_hook_result = _run_hook(pre_hook, hook_phase="pre", metadata=hook_metadata, cwd=tool_cwd)
            _record_hook_failure(runtime, command=pre_hook, hook_phase="pre", result=pre_hook_result)
            if pre_hook_result.returncode != 0:
                _write_output_artifact(runtime, "")
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    failure_reason=_pre_hook_failure_reason(pre_hook, pre_hook_result),
                    context_updates={
                        "context.tool.output": "",
                        "context.tool.exit_code": -1,
                    },
                )
        post_hook = _resolve_hook_command(runtime, "tool.hooks.post")

        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        stdout_text = ""
        stderr_text = ""
        outcome: Outcome
        try:
            proc = subprocess.run(
                command,
                shell=True,
                capture_output=True,
                text=True,
                timeout=timeout,
                cwd=str(tool_cwd),
            )
            stdout_text = proc.stdout or ""
            stderr_text = proc.stderr or ""
            _write_output_artifact(runtime, stdout_text)
            if proc.returncode == 0:
                notes = stdout_text.strip()
                outcome = Outcome(
                    status=OutcomeStatus.SUCCESS,
                    notes=notes,
                    context_updates={
                        "context.tool.output": notes,
                        "context.tool.exit_code": proc.returncode,
                    },
                )
            else:
                reason = stderr_text.strip() or f"tool command failed with code {proc.returncode}"
                outcome = Outcome(
                    status=OutcomeStatus.FAIL,
                    failure_reason=reason,
                    context_updates={
                        "context.tool.output": stdout_text.strip(),
                        "context.tool.exit_code": proc.returncode,
                    },
                )
        except subprocess.TimeoutExpired as exc:
            stdout_text = str(exc.stdout or "")
            stderr_text = str(exc.stderr or "")
            _write_output_artifact(runtime, stdout_text)
            reason = str(exc) or "tool command timed out"
            outcome = Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "context.tool.output": stdout_text.strip(),
                    "context.tool.exit_code": -1,
                },
            )
        except Exception as exc:
            _write_output_artifact(runtime, "")
            reason = str(exc) or "tool command execution error"
            outcome = Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                context_updates={
                    "context.tool.output": "",
                    "context.tool.exit_code": -1,
                },
            )

        if post_hook:
            post_hook_result = _run_hook(post_hook, hook_phase="post", metadata=hook_metadata, cwd=tool_cwd)
            _record_hook_failure(runtime, command=post_hook, hook_phase="post", result=post_hook_result)

        artifact_failure = _capture_declared_artifacts(
            runtime,
            cwd=tool_cwd,
            stdout_text=stdout_text,
            stderr_text=stderr_text,
        )
        if artifact_failure:
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=artifact_failure,
                context_updates=dict(outcome.context_updates or {}),
            )

        return outcome


def _resolve_hook_command(runtime: HandlerRuntime, key: str) -> str:
    node_attr = runtime.node_attrs.get(key)
    if node_attr and str(node_attr.value).strip():
        return str(node_attr.value).strip()

    graph_attr = runtime.graph.graph_attrs.get(key)
    if graph_attr and str(graph_attr.value).strip():
        return str(graph_attr.value).strip()

    return ""


def _run_hook(
    command: str,
    *,
    hook_phase: str,
    metadata: dict[str, str],
    cwd: Path,
) -> subprocess.CompletedProcess[str]:
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
            cwd=str(cwd),
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


def _pre_hook_failure_reason(command: str, result: subprocess.CompletedProcess[str]) -> str:
    stderr = str(result.stderr or "").strip()
    if stderr:
        return f"tool pre-hook blocked execution: {stderr}"
    return f"tool pre-hook blocked execution (exit code {int(result.returncode)}): {command}"


def _write_output_artifact(runtime: HandlerRuntime, output: str) -> None:
    if not runtime.logs_root:
        return
    try:
        stage_dir = runtime.logs_root / runtime.node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        (stage_dir / "tool_output.txt").write_text(output, encoding="utf-8")
    except OSError:
        return


def _resolve_tool_cwd(runtime: HandlerRuntime) -> Path:
    raw_value = str(runtime.context.get("internal.run_workdir", "")).strip()
    if raw_value:
        return Path(raw_value).expanduser().resolve()
    return Path.cwd().resolve()


def _capture_declared_artifacts(
    runtime: HandlerRuntime,
    *,
    cwd: Path,
    stdout_text: str,
    stderr_text: str,
) -> str | None:
    artifact_paths = _artifact_patterns(runtime.node_attrs.get("tool.artifacts.paths"))
    stdout_path = _artifact_relative_path(runtime.node_attrs.get("tool.artifacts.stdout"))
    stderr_path = _artifact_relative_path(runtime.node_attrs.get("tool.artifacts.stderr"))
    if not artifact_paths and not stdout_path and not stderr_path:
        return None
    if runtime.artifact_store is None:
        return "artifact capture unavailable: runtime does not expose an artifact store"

    try:
        if stdout_path:
            runtime.artifact_store.write_text(runtime.node_id, stdout_path, stdout_text)
        if stderr_path:
            runtime.artifact_store.write_text(runtime.node_id, stderr_path, stderr_text)
        if artifact_paths:
            runtime.artifact_store.copy_matches(runtime.node_id, cwd, artifact_paths)
        return None
    except (OSError, ValueError, RuntimeError) as exc:
        reason = str(exc).strip() or exc.__class__.__name__
        return f"artifact capture failed: {reason}"


def _artifact_patterns(attr: Any) -> list[str]:
    if not attr:
        return []
    raw_value = str(getattr(attr, "value", "")).strip()
    if not raw_value:
        return []
    return [segment.strip() for segment in raw_value.split(",") if segment.strip()]


def _artifact_relative_path(attr: Any) -> str:
    if not attr:
        return ""
    return str(getattr(attr, "value", "")).strip()


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
