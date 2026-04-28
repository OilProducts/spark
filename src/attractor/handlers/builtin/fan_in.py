from __future__ import annotations

from contextlib import nullcontext
import inspect
import json
from pathlib import Path
from typing import Any, Dict, List, Optional

from attractor.dsl.models import Duration
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.llm_runtime import (
    resolve_effective_llm_model,
    resolve_effective_llm_profile,
    resolve_effective_llm_provider,
    resolve_effective_reasoning_effort,
)

from ..base import CodergenBackend, HandlerRuntime


class FanInHandler:
    def __init__(self, backend: Optional[CodergenBackend] = None):
        self.backend = backend

    def execute(self, runtime: HandlerRuntime) -> Outcome:
        raw_results = runtime.context.get("parallel.results", [])
        results = _normalize_results(raw_results)
        if not results:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No parallel results to evaluate")

        candidates = [r for r in results if _status_rank(_result_status(r)) < _status_rank("fail")]
        if not candidates:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="All parallel branches failed")

        best = None
        if runtime.prompt.strip() and self.backend is not None:
            best = _backend_select(
                self.backend,
                runtime.node_id,
                runtime.prompt,
                runtime.context,
                runtime.node_attrs,
                runtime.node_attrs.get("timeout"),
                runtime.logs_root,
                candidates,
            )
        if best is None:
            best = sorted(
                candidates,
                key=lambda r: (
                    _status_rank(_result_status(r)),
                    -_score_value(r.get("score")),
                    str(r.get("id", "")),
                ),
            )[0]

        return Outcome(
            status=OutcomeStatus.SUCCESS,
            context_updates={
                "parallel.fan_in.best_id": best.get("id", ""),
                "parallel.fan_in.best_outcome": _result_status(best),
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


def _result_status(result: Dict[str, Any]) -> str:
    status = str(result.get("status", "") or "").strip().lower()
    outcome = str(result.get("outcome", "") or "").strip().lower()
    if status == "completed" and outcome:
        return outcome
    if status == "failed":
        return "fail"
    return status


def _score_value(score: Any) -> float:
    if score is None:
        return 0.0
    if isinstance(score, (int, float)):
        return float(score)
    try:
        return float(str(score))
    except (TypeError, ValueError):
        return 0.0


def _backend_select(
    backend: CodergenBackend,
    node_id: str,
    prompt: str,
    context,
    node_attrs,
    timeout_attr: Any,
    logs_root: Path | None,
    candidates: List[Dict[str, Any]],
) -> Dict[str, Any] | None:
    ranking_prompt = _build_ranking_prompt(prompt, candidates)
    timeout = _to_seconds(timeout_attr)
    effective_model = resolve_effective_llm_model(
        node_attrs,
        context,
        fallback_model=getattr(backend, "model", None),
    )
    effective_provider = resolve_effective_llm_provider(
        node_attrs,
        context,
        fallback_provider=getattr(backend, "provider", None),
    )
    effective_profile = resolve_effective_llm_profile(node_attrs, context)
    effective_reasoning_effort = resolve_effective_reasoning_effort(
        node_attrs,
        context,
        fallback_reasoning_effort=getattr(backend, "reasoning_effort", None),
    )
    with _backend_stage_logging_context(backend, node_id, logs_root):
        backend_kwargs = {"timeout": timeout, "provider": effective_provider, "llm_profile": effective_profile}
        if effective_model is not None:
            backend_kwargs["model"] = effective_model
        if effective_reasoning_effort is not None:
            backend_kwargs["reasoning_effort"] = effective_reasoning_effort
        backend_kwargs = _filter_backend_kwargs(backend.run, backend_kwargs)
        response = backend.run(node_id, ranking_prompt, context, **backend_kwargs)
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


def _backend_stage_logging_context(backend: object, node_id: str, logs_root: Path | None):
    binder = getattr(backend, "bind_stage_raw_rpc_log", None)
    if callable(binder):
        return binder(node_id, logs_root)
    return nullcontext()


def _filter_backend_kwargs(callable_obj, kwargs: dict) -> dict:
    try:
        parameters = inspect.signature(callable_obj).parameters
    except (TypeError, ValueError):
        return kwargs
    if any(parameter.kind == inspect.Parameter.VAR_KEYWORD for parameter in parameters.values()):
        return kwargs
    return {key: value for key, value in kwargs.items() if key in parameters}


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
