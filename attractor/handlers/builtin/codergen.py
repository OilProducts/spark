from __future__ import annotations

from pathlib import Path
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
        stage_dir = _ensure_stage_dir(runtime.logs_root, runtime.node_id)
        _write_stage_file(stage_dir, "prompt.md", prompt)

        if self.backend is None:
            outcome = Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="codergen handler completed without backend",
            )
            _write_stage_file(stage_dir, "response.md", outcome.notes or "")
            return outcome

        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        result = self.backend.run(runtime.node_id, prompt, runtime.context, timeout=timeout)
        outcome: Outcome
        response_text: str
        if isinstance(result, Outcome):
            outcome = result
            response_text = outcome.notes or outcome.failure_reason or ""
        elif isinstance(result, str):
            response_text = result
            outcome = Outcome(status=OutcomeStatus.SUCCESS, notes="codergen backend success")
        elif result:
            outcome = Outcome(status=OutcomeStatus.SUCCESS, notes="codergen backend success")
            response_text = outcome.notes
        else:
            outcome = Outcome(status=OutcomeStatus.FAIL, failure_reason="codergen backend failure")
            response_text = outcome.failure_reason
        _write_stage_file(stage_dir, "response.md", response_text)
        return outcome


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


def _ensure_stage_dir(logs_root: Path | None, node_id: str) -> Path | None:
    if logs_root is None:
        return None
    stage_dir = logs_root / node_id
    stage_dir.mkdir(parents=True, exist_ok=True)
    return stage_dir


def _write_stage_file(stage_dir: Path | None, filename: str, content: str) -> None:
    if stage_dir is None:
        return
    (stage_dir / filename).write_text(content + "\n", encoding="utf-8")
