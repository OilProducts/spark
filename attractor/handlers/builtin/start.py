from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class StartHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        return Outcome(status=OutcomeStatus.SUCCESS, notes="start no-op")
