from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

from attractor.dsl.models import Duration
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import CodergenBackend, HandlerRuntime


class FanInHandler:
    def __init__(self, backend: Optional[CodergenBackend] = None):
        self.backend = backend

    def execute(self, runtime: HandlerRuntime) -> Outcome:
        raw_results = runtime.context.get("parallel.results", [])
        results = _normalize_results(raw_results)
        if not results:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No parallel results to evaluate")

        candidates = [r for r in results if _status_rank(r.get("status", "")) < _status_rank("fail")]
        if not candidates:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="All parallel branches failed")

        best = None
        if runtime.prompt.strip() and self.backend is not None:
            best = _backend_select(
                self.backend,
                runtime.node_id,
                runtime.prompt,
                runtime.context,
                runtime.node_attrs.get("timeout"),
                candidates,
            )
        if best is None:
            best = sorted(
                candidates,
                key=lambda r: (_status_rank(r.get("status", "")), str(r.get("id", ""))),
            )[0]

        return Outcome(
            status=OutcomeStatus.SUCCESS,
            context_updates={
                "parallel.fan_in.best_id": best.get("id", ""),
                "parallel.fan_in.best_outcome": best.get("status", ""),
            },
            notes=f"Selected best candidate: {best.get('id', '')}",
        )


def _normalize_results(raw: Any) -> List[Dict[str, Any]]:
    if isinstance(raw, list):
        return [r for r in raw if isinstance(r, dict)]
    if isinstance(raw, str):
        try:
            parsed = json.loads(raw)
            if isinstance(parsed, list):
                return [r for r in parsed if isinstance(r, dict)]
        except json.JSONDecodeError:
            return []
    return []


def _status_rank(status: str) -> int:
    normalized = str(status or "").lower()
    order = {
        "success": 0,
        "partial_success": 1,
        "paused": 1,
        "retry": 2,
        "fail": 3,
    }
    return order.get(normalized, 4)


def _backend_select(
    backend: CodergenBackend,
    node_id: str,
    prompt: str,
    context,
    timeout_attr: Any,
    candidates: List[Dict[str, Any]],
) -> Dict[str, Any] | None:
    ranking_prompt = _build_ranking_prompt(prompt, candidates)
    timeout = _to_seconds(timeout_attr)
    response = backend.run(node_id, ranking_prompt, context, timeout=timeout)
    best_id = _extract_best_id(response)
    if not best_id:
        return None

    for candidate in candidates:
        if str(candidate.get("id", "")) == best_id:
            return candidate
    return None


def _build_ranking_prompt(prompt: str, candidates: List[Dict[str, Any]]) -> str:
    payload = json.dumps(candidates, ensure_ascii=True, separators=(",", ":"))
    return (
        f"{prompt.strip()}\n\n"
        "Select the best candidate from parallel execution results.\n"
        "Return JSON only in the form {\"best_id\":\"<candidate id>\"}.\n"
        f"Candidates: {payload}"
    )


def _extract_best_id(response: str | Outcome) -> str:
    if isinstance(response, Outcome):
        direct = str(response.context_updates.get("parallel.fan_in.best_id", "")).strip()
        if direct:
            return direct
        direct = str(response.context_updates.get("best_id", "")).strip()
        if direct:
            return direct
        return _extract_best_id_from_text(response.notes)
    if isinstance(response, str):
        return _extract_best_id_from_text(response)
    return ""


def _extract_best_id_from_text(text: str) -> str:
    raw = str(text or "").strip()
    if not raw:
        return ""
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return raw
    if isinstance(parsed, dict):
        value = parsed.get("best_id", parsed.get("id", ""))
        return str(value).strip()
    if isinstance(parsed, str):
        return parsed.strip()
    return ""


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
