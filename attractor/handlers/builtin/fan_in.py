from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class FanInHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        return Outcome(status=OutcomeStatus.SUCCESS, notes="fan-in pass-through")
