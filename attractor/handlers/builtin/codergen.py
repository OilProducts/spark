from __future__ import annotations

from typing import Optional

from attractor.dsl.models import Duration

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import CodergenBackend, HandlerRuntime


class CodergenHandler:
    def __init__(self, backend: Optional[CodergenBackend] = None):
        self.backend = backend

    def execute(self, runtime: HandlerRuntime) -> Outcome:
        prompt = runtime.prompt.strip()
        if not prompt:
            label_attr = runtime.node_attrs.get("label")
            if label_attr:
                prompt = str(label_attr.value).strip()
            if not prompt:
                prompt = runtime.node_id
        prompt = _expand_goal(prompt, runtime.context, runtime.graph)
        if self.backend is None:
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="codergen handler completed without backend",
            )

        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        ok = self.backend.run(runtime.node_id, prompt, runtime.context, timeout=timeout)
        if ok:
            return Outcome(status=OutcomeStatus.SUCCESS, notes="codergen backend success")
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="codergen backend failure")


def _expand_goal(prompt: str, context, graph) -> str:
    goal = context.get("graph.goal")
    if goal in (None, ""):
        goal_attr = graph.graph_attrs.get("goal")
        if goal_attr is not None:
            goal = goal_attr.value
    if goal is None:
        goal = ""
    return prompt.replace("$goal", str(goal))


def _to_seconds(attr) -> float | None:
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
