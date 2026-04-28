from __future__ import annotations

from contextlib import nullcontext
import inspect
import json
from pathlib import Path
from typing import Optional

from attractor.dsl.models import Duration
from attractor.engine.context_contracts import (
    ContextReadContract,
    resolve_context_read_contract,
    resolve_context_write_contract,
)
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.engine.status_envelope_prompting import build_status_envelope_prompt_appendix
from attractor.llm_runtime import (
    resolve_effective_llm_model,
    resolve_effective_llm_profile,
    resolve_effective_llm_provider,
    resolve_effective_reasoning_effort,
)

from ..base import CodergenBackend, HandlerRuntime

RUNTIME_CONTEXT_CARRYOVER_KEY = "_attractor.runtime.context_carryover"
DECLARED_CONTEXT_MISSING_SENTINEL = "<missing>"
STATUS_ENVELOPE_RESPONSE_CONTRACT = "status_envelope"
DEFAULT_CONTRACT_REPAIR_ATTEMPTS = 1


class CodergenHandler:
    def __init__(self, backend: Optional[CodergenBackend] = None):
        self.backend = backend

    def execute(self, runtime: HandlerRuntime) -> Outcome:
        stage_dir = _ensure_stage_dir(runtime.logs_root, runtime.node_id)
        prompt = runtime.prompt.strip()
        if not prompt:
            label_attr = runtime.node_attrs.get("label")
            if label_attr:
                prompt = str(label_attr.value).strip()
            if not prompt:
                prompt = runtime.node_id
        read_contract = resolve_context_read_contract(runtime.node_attrs)
        if read_contract.parse_error:
            failure_reason = f"spark.reads_context parse error: {read_contract.parse_error}"
            outcome = _with_builtin_response_context(
                Outcome(
                    status=OutcomeStatus.FAIL,
                    failure_reason=failure_reason,
                    retryable=False,
                    failure_kind=FailureKind.CONTRACT,
                ),
                node_id=runtime.node_id,
                response_text=failure_reason,
            )
            _write_stage_file(stage_dir, "response.md", failure_reason)
            _write_status_file(stage_dir, outcome)
            return outcome
        write_contract = resolve_context_write_contract(runtime.node_attrs)
        prompt = _expand_goal(prompt, runtime.context, runtime.graph)
        prompt = _apply_response_contract(prompt, runtime.node_attrs, write_contract)
        prompt = _compose_prompt(prompt, runtime.context, read_contract)
        _write_stage_file(stage_dir, "prompt.md", prompt)

        if self.backend is None:
            response_text = f"[Simulated] Response for stage: {runtime.node_id}"
            outcome = _with_builtin_response_context(
                Outcome(
                    status=OutcomeStatus.SUCCESS,
                    notes=f"Stage completed: {runtime.node_id}",
                ),
                node_id=runtime.node_id,
                response_text=response_text,
            )
            _write_stage_file(stage_dir, "response.md", response_text)
            _write_status_file(stage_dir, outcome)
            return outcome

        timeout = _to_seconds(runtime.node_attrs.get("timeout"))
        response_contract = _normalized_response_contract_name(runtime.node_attrs)
        contract_repair_attempts = _contract_repair_attempts(runtime.node_attrs, response_contract)
        effective_model = resolve_effective_llm_model(
            runtime.node_attrs,
            runtime.context,
            fallback_model=getattr(self.backend, "model", None) if self.backend is not None else None,
        )
        effective_provider = resolve_effective_llm_provider(
            runtime.node_attrs,
            runtime.context,
            fallback_provider=getattr(self.backend, "provider", None) if self.backend is not None else None,
        )
        effective_profile = resolve_effective_llm_profile(runtime.node_attrs, runtime.context)
        effective_reasoning_effort = resolve_effective_reasoning_effort(
            runtime.node_attrs,
            runtime.context,
            fallback_reasoning_effort=getattr(self.backend, "reasoning_effort", None)
            if self.backend is not None
            else None,
        )
        backend_kwargs = {
            "response_contract": response_contract,
            "contract_repair_attempts": contract_repair_attempts,
            "timeout": timeout,
            "write_contract": write_contract,
            "provider": effective_provider,
            "llm_profile": effective_profile,
        }
        if effective_model is not None:
            backend_kwargs["model"] = effective_model
        if effective_reasoning_effort is not None:
            backend_kwargs["reasoning_effort"] = effective_reasoning_effort
        backend_kwargs = _filter_backend_kwargs(self.backend.run, backend_kwargs)
        with _backend_stage_logging_context(self.backend, runtime.node_id, runtime.logs_root):
            result = self.backend.run(
                runtime.node_id,
                prompt,
                runtime.context,
                emit_event=runtime.event_emitter,
                **backend_kwargs,
            )
        outcome: Outcome
        response_text: str
        if isinstance(result, Outcome):
            response_text = _response_text_for_outcome(result)
            outcome = _with_builtin_response_context(
                result,
                node_id=runtime.node_id,
                response_text=response_text,
            )
        elif isinstance(result, str):
            response_text = result
            outcome = _with_builtin_response_context(
                Outcome(status=OutcomeStatus.SUCCESS, notes=response_text),
                node_id=runtime.node_id,
                response_text=response_text,
            )
        elif result:
            outcome = _with_builtin_response_context(
                Outcome(status=OutcomeStatus.SUCCESS, notes="codergen backend success"),
                node_id=runtime.node_id,
                response_text="codergen backend success",
            )
            response_text = outcome.notes
        else:
            outcome = _with_builtin_response_context(
                Outcome(status=OutcomeStatus.FAIL, failure_reason="codergen backend failure"),
                node_id=runtime.node_id,
                response_text="codergen backend failure",
            )
            response_text = outcome.failure_reason
        _write_stage_file(stage_dir, "response.md", response_text)
        _write_status_file(stage_dir, outcome)
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


def _compose_prompt(prompt: str, context, read_contract: ContextReadContract) -> str:
    sections: list[str] = []
    carryover = str(context.get(RUNTIME_CONTEXT_CARRYOVER_KEY, "") or "").strip()
    if carryover:
        sections.extend(["Context carryover:", carryover])
    declared_context_reads = _render_declared_context_reads(read_contract, context)
    if declared_context_reads:
        sections.extend(["Declared context reads:", declared_context_reads])
    if not sections:
        return prompt
    sections.extend(["Current stage task:", prompt])
    return "\n\n".join(sections)


def _render_declared_context_reads(read_contract: ContextReadContract, context) -> str:
    if not read_contract.declared_keys:
        return ""
    snapshot = context.snapshot()
    lines = []
    for key in read_contract.declared_keys:
        if key in snapshot:
            lines.append(f"{key}={_to_context_text(snapshot[key])}")
        else:
            lines.append(f"{key}={DECLARED_CONTEXT_MISSING_SENTINEL}")
    return "\n".join(lines)


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


def _apply_response_contract(prompt: str, node_attrs, write_contract) -> str:
    contract_name = _normalized_response_contract_name(node_attrs)
    if contract_name != STATUS_ENVELOPE_RESPONSE_CONTRACT:
        return prompt
    return "\n\n".join([prompt, build_status_envelope_prompt_appendix(write_contract)])


def _normalized_response_contract_name(node_attrs) -> str:
    attr = node_attrs.get("codergen.response_contract")
    if attr is None:
        return ""
    return str(attr.value).strip().lower().replace("-", "_")


def _contract_repair_attempts(node_attrs, response_contract: str) -> int:
    if not response_contract:
        return 0
    attr = node_attrs.get("codergen.contract_repair_attempts")
    if attr is None:
        return DEFAULT_CONTRACT_REPAIR_ATTEMPTS
    try:
        return max(0, int(str(attr.value).strip()))
    except (TypeError, ValueError):
        return DEFAULT_CONTRACT_REPAIR_ATTEMPTS


def _with_builtin_response_context(outcome: Outcome, *, node_id: str, response_text: str) -> Outcome:
    merged_updates = {
        "last_stage": node_id,
        "last_response": response_text[:200],
    }
    merged_updates.update(dict(outcome.context_updates))
    outcome.context_updates = merged_updates
    return outcome


def _response_text_for_outcome(outcome: Outcome) -> str:
    raw_response_text = getattr(outcome, "raw_response_text", "")
    if raw_response_text:
        return str(raw_response_text)
    if outcome.notes:
        return str(outcome.notes)
    if outcome.failure_reason:
        return str(outcome.failure_reason)
    return ""


def _to_context_text(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float, str)):
        return str(value)
    return json.dumps(value, sort_keys=True)


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


def _write_status_file(stage_dir: Path | None, outcome: Outcome) -> None:
    if stage_dir is None:
        return
    payload = {
        "outcome": outcome.status.value,
        "preferred_label": outcome.preferred_label,
        "suggested_next_ids": list(outcome.suggested_next_ids),
        "context_updates": dict(outcome.context_updates),
        "notes": outcome.notes,
    }
    if outcome.status == OutcomeStatus.FAIL and outcome.failure_kind is not None:
        payload["failure_kind"] = outcome.failure_kind.value
    with (stage_dir / "status.json").open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
