from __future__ import annotations

from typing import Optional

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import CodergenBackend, HandlerRuntime


class CodergenHandler:
    def __init__(self, backend: Optional[CodergenBackend] = None):
        self.backend = backend

    def run(self, runtime: HandlerRuntime) -> Outcome:
        prompt = _expand_goal(runtime.prompt, runtime.context)
        if self.backend is None:
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="codergen handler completed without backend",
            )

        ok = self.backend.run(runtime.node_id, prompt, runtime.context)
        if ok:
            return Outcome(status=OutcomeStatus.SUCCESS, notes="codergen backend success")
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="codergen backend failure")


def _expand_goal(prompt: str, context) -> str:
    goal = context.get("graph.goal", "")
    return prompt.replace("$goal", str(goal))
