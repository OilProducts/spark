from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class ParallelHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        suggested = [edge.target for edge in runtime.outgoing_edges]
        return Outcome(
            status=OutcomeStatus.SUCCESS,
            suggested_next_ids=suggested,
            notes="parallel fan-out staged",
        )
