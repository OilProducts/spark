from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ConditionalHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        return Outcome(status=OutcomeStatus.SUCCESS, notes="conditional pass-through")
