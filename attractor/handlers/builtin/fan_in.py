from __future__ import annotations

import json
from typing import Any, Dict, List

from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


class FanInHandler:
    def run(self, runtime: HandlerRuntime) -> Outcome:
        raw_results = runtime.context.get("parallel.results", [])
        results = _normalize_results(raw_results)
        if not results:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No parallel results to evaluate")

        candidates = [r for r in results if _status_rank(r.get("status", "")) < _status_rank("fail")]
        if not candidates:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="All parallel branches failed")

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
