from __future__ import annotations

import subprocess

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ToolHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        cmd_attr = runtime.node_attrs.get("tool_command")
        if not cmd_attr or not str(cmd_attr.value).strip():
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No tool_command specified")

        command = str(cmd_attr.value)
        proc = subprocess.run(command, shell=True, capture_output=True, text=True)
        if proc.returncode == 0:
            notes = proc.stdout.strip()
            return Outcome(status=OutcomeStatus.SUCCESS, notes=notes)

        reason = proc.stderr.strip() or f"tool command failed with code {proc.returncode}"
        return Outcome(status=OutcomeStatus.FAIL, failure_reason=reason)
