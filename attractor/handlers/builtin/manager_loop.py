from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ManagerLoopHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason="stack.manager_loop not implemented",
        )
