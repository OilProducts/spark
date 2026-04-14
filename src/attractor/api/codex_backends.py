from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime
import json
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Callable, Optional

from attractor.api.token_usage import (
    TokenUsageBreakdown,
    TokenUsageBucket,
    compute_live_usage_delta,
)
from attractor.engine.context import Context
from attractor.engine.context_contracts import (
    ContextWriteContract,
    validate_context_updates_against_contract,
)
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.engine.status_envelope_prompting import (
    build_status_envelope_context_updates_contract_text,
    format_status_envelope_allowed_keys,
)
from attractor.handlers.base import CodergenBackend
from spark_common.codex_app_client import CodexAppServerClient
from spark_common import codex_app_server
from spark_common.runtime import resolve_runtime_workspace_path

_STRUCTURED_OUTCOME_KEYS = {
    "outcome",
    "preferred_label",
    "suggested_next_ids",
    "context_updates",
    "notes",
    "failure_reason",
    "retryable",
}


@dataclass(frozen=True)
class _PlainTextParseResult:
    raw_text: str


@dataclass(frozen=True)
class _ModeledOutcomeParseResult:
    outcome: Outcome


@dataclass(frozen=True)
class _StructuredContractViolation:
    response_contract: str
    raw_text: str
    reason: str
    write_contract: ContextWriteContract | None = None


class CodexAppServerBackend(CodergenBackend):
    RUNTIME_THREAD_ID_KEY = "_attractor.runtime.thread_id"

    def __init__(
        self,
        working_dir: str,
        emit,
        model: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
    ):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.model = model
        self._on_usage_update = on_usage_update
        self._session_threads_by_key: dict[str, str] = {}
        self._session_threads_lock = threading.Lock()
        self._raw_rpc_log_lock = threading.Lock()
        self._raw_rpc_log_state = threading.local()
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()

    @contextmanager
    def bind_stage_raw_rpc_log(self, node_id: str, logs_root: str | Path | None):
        previous = getattr(self._raw_rpc_log_state, "path", None)
        self._raw_rpc_log_state.path = self._stage_raw_rpc_log_path(node_id, logs_root)
        try:
            yield
        finally:
            if previous is None:
                if hasattr(self._raw_rpc_log_state, "path"):
                    delattr(self._raw_rpc_log_state, "path")
            else:
                self._raw_rpc_log_state.path = previous

    def _stage_raw_rpc_log_path(self, node_id: str, logs_root: str | Path | None) -> Path | None:
        if logs_root is None:
            return None
        stage_dir = Path(logs_root) / node_id
        stage_dir.mkdir(parents=True, exist_ok=True)
        return stage_dir / "raw-rpc.jsonl"

    def _append_raw_rpc_log(self, direction: str, line: str) -> None:
        path = getattr(self._raw_rpc_log_state, "path", None)
        if path is None:
            return
        payload = {
            "timestamp": datetime.now(UTC).isoformat(),
            "direction": direction,
            "line": line,
        }
        with self._raw_rpc_log_lock:
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(payload, sort_keys=True) + "\n")

    def _runtime_thread_key(self, context: Context) -> str:
        value = context.get(self.RUNTIME_THREAD_ID_KEY, "")
        if value is None:
            return ""
        return str(value).strip()

    def _resolve_session_thread_id(
        self,
        thread_key: str,
        model: Optional[str],
        start_thread: Callable[[], str | None],
    ) -> str | None:
        normalized_key = thread_key.strip()
        if not normalized_key:
            return start_thread()

        cache_key = normalized_key
        normalized_model = str(model or "").strip()
        if normalized_model:
            cache_key = f"{normalized_key}::{normalized_model}"

        with self._session_threads_lock:
            cached = self._session_threads_by_key.get(cache_key)
            if cached:
                return cached
            created = start_thread()
            if not created:
                return None
            self._session_threads_by_key[cache_key] = created
            return created

    def _record_token_usage_delta(self, *, model: Optional[str], delta: TokenUsageBucket) -> None:
        if not delta.has_any_usage():
            return
        normalized_model = str(model or "").strip() or "codex default (config/profile)"
        with self._token_usage_lock:
            self._token_usage_breakdown.add_for_model(normalized_model, delta)
            snapshot = self._token_usage_breakdown.copy()
        if self._on_usage_update is not None:
            self._on_usage_update(snapshot)

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: Optional[float] = None,
        model: Optional[str] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                failure_kind=FailureKind.RUNTIME,
            )

        client = CodexAppServerClient(
            self.working_dir,
            requested_working_dir=self.requested_working_dir,
            on_unparsed_line=log_line,
        )
        effective_model = str(model or "").strip() or self.model
        set_raw_rpc_logger = getattr(client, "set_raw_rpc_logger", None)
        clear_raw_rpc_logger = getattr(client, "clear_raw_rpc_logger", None)
        try:
            if callable(set_raw_rpc_logger):
                set_raw_rpc_logger(self._append_raw_rpc_log)
            client.ensure_process(popen_factory=subprocess.Popen)

            def start_thread() -> str | None:
                try:
                    return client.start_thread(
                        model=effective_model,
                        cwd=self.working_dir,
                        ephemeral=True,
                    )
                except RuntimeError:
                    return None

            thread_key = self._runtime_thread_key(context)
            thread_uuid = self._resolve_session_thread_id(thread_key, effective_model, start_thread)
            if not thread_uuid:
                return fail("codex app-server thread/start failed")

            turn_text = self._run_turn_and_capture_text(
                client,
                thread_uuid,
                prompt,
                timeout,
                log_line,
                model=effective_model,
            )
            if turn_text is None:
                return "codex app-server completed successfully"
            return self._coerce_or_repair_contract_result(
                client,
                thread_uuid,
                node_id,
                turn_text,
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                timeout=timeout,
                log_line=log_line,
                model=effective_model,
                write_contract=write_contract,
            )
        except RuntimeError as exc:
            return fail(str(exc))
        finally:
            if callable(clear_raw_rpc_logger):
                clear_raw_rpc_logger()
            client.close()

    def _run_turn_and_capture_text(
        self,
        client: CodexAppServerClient,
        thread_id: str,
        prompt: str,
        timeout: Optional[float],
        log_line: Callable[[str], None],
        *,
        model: Optional[str],
    ) -> str | None:
        previous_total: TokenUsageBucket | None = None
        saw_usage_update = False

        def handle_turn_event(event: codex_app_server.CodexAppServerTurnEvent) -> None:
            nonlocal previous_total, saw_usage_update
            if event.kind != "token_usage_updated" or event.token_usage is None:
                return
            delta, previous_total = compute_live_usage_delta(event.token_usage, previous_total)
            if delta is None or not delta.has_any_usage():
                return
            saw_usage_update = True
            self._record_token_usage_delta(model=model, delta=delta)

        result = client.run_turn(
            thread_id=thread_id,
            prompt=prompt,
            model=model,
            cwd=self.working_dir,
            on_event=handle_turn_event,
            overall_timeout_seconds=timeout,
            now=time.monotonic,
        )
        if not saw_usage_update:
            delta, _ = compute_live_usage_delta(getattr(result, "token_usage_payload", None), None)
            if delta is not None and delta.has_any_usage():
                self._record_token_usage_delta(model=model, delta=delta)
        agent_text = result.assistant_message
        if agent_text:
            log_line(agent_text)
        command_text = result.command_text
        if command_text:
            log_line(command_text)
        if result.token_total is not None:
            log_line(f"tokens used: {result.token_total}")
        return agent_text or command_text or None

    def _coerce_or_repair_contract_result(
        self,
        client: CodexAppServerClient,
        thread_id: str,
        node_id: str,
        response_text: str,
        *,
        response_contract: str,
        contract_repair_attempts: int,
        timeout: Optional[float],
        log_line: Callable[[str], None],
        model: Optional[str],
        write_contract: ContextWriteContract | None,
    ) -> str | Outcome:
        result = _coerce_structured_text_outcome(response_text, response_contract=response_contract)
        if isinstance(result, Outcome):
            return result
        if isinstance(result, _ModeledOutcomeParseResult):
            violation = _validate_write_contract_violation(
                result.outcome,
                write_contract=write_contract,
                response_contract=response_contract,
                raw_text=response_text,
            )
            if violation is None:
                return result.outcome
            result = violation
        if isinstance(result, _PlainTextParseResult):
            return result.raw_text

        if contract_repair_attempts <= 0:
            return _contract_failure_outcome(result)

        current_violation = _with_write_contract(result, write_contract)
        for attempt in range(1, contract_repair_attempts + 1):
            log_line(
                f"response contract violation for {node_id}; requesting corrected final answer "
                f"(attempt {attempt}/{contract_repair_attempts}): {current_violation.reason}"
            )
            repair_prompt = _build_contract_repair_prompt(current_violation)
            repair_text = self._run_turn_and_capture_text(
                client,
                thread_id,
                repair_prompt,
                timeout,
                log_line,
                model=model,
            )
            if repair_text is None:
                return _contract_failure_outcome(current_violation)
            repaired = _coerce_structured_text_outcome(
                repair_text,
                response_contract=current_violation.response_contract,
            )
            if isinstance(repaired, _ModeledOutcomeParseResult):
                repaired_violation = _validate_write_contract_violation(
                    repaired.outcome,
                    write_contract=write_contract,
                    response_contract=current_violation.response_contract,
                    raw_text=repair_text,
                )
                if repaired_violation is None:
                    return repaired.outcome
                current_violation = repaired_violation
                continue
            if isinstance(repaired, _PlainTextParseResult):
                return repaired.raw_text
            current_violation = _with_write_contract(repaired, write_contract)
        return _contract_failure_outcome(current_violation)


def _validate_write_contract_violation(
    outcome: Outcome,
    *,
    write_contract: ContextWriteContract | None,
    response_contract: str,
    raw_text: str,
) -> _StructuredContractViolation | None:
    if not _has_response_contract(response_contract) or write_contract is None:
        return None
    violation = validate_context_updates_against_contract(outcome.context_updates, write_contract)
    if violation is None:
        return None
    return _StructuredContractViolation(
        response_contract=response_contract,
        raw_text=raw_text.strip(),
        reason=violation.format_reason(),
        write_contract=write_contract,
    )


def _with_write_contract(
    violation: _StructuredContractViolation,
    write_contract: ContextWriteContract | None,
) -> _StructuredContractViolation:
    if violation.write_contract is write_contract:
        return violation
    return _StructuredContractViolation(
        response_contract=violation.response_contract,
        raw_text=violation.raw_text,
        reason=violation.reason,
        write_contract=write_contract,
    )


def build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
    on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
) -> CodergenBackend:
    normalized = backend_name.strip().lower()
    if normalized == "codex-app-server":
        return CodexAppServerBackend(working_dir, emit, model=model, on_usage_update=on_usage_update)
    raise ValueError(
        "Unsupported backend. Supported backends: codex-app-server."
    )


def _coerce_structured_text_outcome(
    text: str,
    *,
    response_contract: str = "",
) -> _PlainTextParseResult | _ModeledOutcomeParseResult | _StructuredContractViolation:
    raw_text = text.strip()
    candidate, envelope_error = _extract_structured_outcome_payload(
        text,
        require_contract=_has_response_contract(response_contract),
    )
    if envelope_error is not None:
        return _contract_violation_or_invalid_outcome(raw_text, envelope_error, response_contract)
    if candidate is None:
        return _PlainTextParseResult(raw_text=raw_text)

    preferred_label = candidate.get("preferred_label", "")
    suggested_next_ids = candidate.get("suggested_next_ids", [])
    context_updates = candidate.get("context_updates", {})
    notes = candidate.get("notes", "")
    failure_reason = candidate.get("failure_reason", "")
    retryable = candidate.get("retryable", None)

    if not isinstance(preferred_label, str):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: preferred_label must be a string",
            response_contract,
        )
    if not isinstance(suggested_next_ids, list) or any(not isinstance(item, str) for item in suggested_next_ids):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: suggested_next_ids must be a list of strings",
            response_contract,
        )
    if not isinstance(context_updates, dict):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: context_updates must be an object",
            response_contract,
        )
    if notes is not None and not isinstance(notes, str):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: notes must be a string",
            response_contract,
        )
    if failure_reason is not None and not isinstance(failure_reason, str):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: failure_reason must be a string",
            response_contract,
        )
    if retryable is not None and not isinstance(retryable, bool):
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: retryable must be a boolean",
            response_contract,
        )

    outcome_name = str(candidate.get("outcome", "")).strip().lower()
    try:
        status = OutcomeStatus(outcome_name)
    except ValueError:
        return _contract_violation_or_invalid_outcome(
            text,
            f"invalid structured status envelope: unsupported outcome status '{outcome_name or '<empty>'}'",
            response_contract,
        )

    if status == OutcomeStatus.SKIPPED:
        return _contract_violation_or_invalid_outcome(
            text,
            "invalid structured status envelope: unsupported outcome status 'skipped'",
            response_contract,
        )

    return _ModeledOutcomeParseResult(
        outcome=Outcome(
            status=status,
            preferred_label=preferred_label,
            suggested_next_ids=list(suggested_next_ids),
            context_updates=dict(context_updates),
            notes=notes or "",
            failure_reason=failure_reason or "",
            retryable=retryable,
            failure_kind=FailureKind.BUSINESS
            if _has_response_contract(response_contract) and status == OutcomeStatus.FAIL
            else None,
            raw_response_text=raw_text,
        )
    )


def _invalid_structured_outcome(text: str, reason: str) -> Outcome:
    return Outcome(
        status=OutcomeStatus.FAIL,
        notes=text.strip(),
        failure_reason=reason,
        raw_response_text=text.strip(),
    )


def _contract_violation_or_invalid_outcome(
    text: str,
    reason: str,
    response_contract: str,
) -> Outcome | _StructuredContractViolation:
    if _has_response_contract(response_contract):
        return _StructuredContractViolation(
            response_contract=response_contract,
            raw_text=text.strip(),
            reason=reason,
        )
    return _invalid_structured_outcome(text, reason)


def _has_response_contract(response_contract: str) -> bool:
    return bool(str(response_contract).strip())


def _build_contract_repair_prompt(violation: _StructuredContractViolation) -> str:
    lines = [
        f"Your previous final answer violated the {violation.response_contract} response contract.",
        f"Validation error: {violation.reason}",
        "",
        "Re-emit only a corrected final answer for the same decision.",
        "Do not do new repository work.",
        "Do not run commands.",
        "Do not change the substantive decision, routing label, or context updates except as required to satisfy the response contract.",
    ]
    allowed_keys = tuple(violation.write_contract.allowed_keys) if violation.write_contract is not None else ()
    if allowed_keys:
        lines.append(
            'Re-emit the same decision using only these "context_updates" keys when needed: '
            f"{format_status_envelope_allowed_keys(violation.write_contract)}."
        )
    else:
        lines.append('Re-emit the same decision with no "context_updates".')
    lines.extend(
        [
            build_status_envelope_context_updates_contract_text(violation.write_contract),
            "",
            "Previous invalid final answer:",
            violation.raw_text,
        ]
    )
    return "\n".join(lines)


def _contract_failure_outcome(violation: _StructuredContractViolation) -> Outcome:
    return Outcome(
        status=OutcomeStatus.FAIL,
        notes=violation.raw_text,
        failure_reason=violation.reason,
        failure_kind=FailureKind.CONTRACT,
        raw_response_text=violation.raw_text,
    )


def _extract_structured_outcome_payload(
    text: str,
    *,
    require_contract: bool = False,
) -> tuple[dict[str, object] | None, str | None]:
    stripped = text.strip()
    if not stripped:
        if require_contract:
            return None, "invalid structured status envelope: empty response"
        return None, None

    candidates = [stripped]
    if stripped.startswith("```") and stripped.endswith("```"):
        lines = stripped.splitlines()
        if len(lines) >= 3:
            inner = "\n".join(lines[1:-1]).strip()
            if inner and inner not in candidates:
                candidates.append(inner)

    validation_errors: list[str] = []
    for candidate in candidates:
        try:
            payload = json.loads(candidate)
        except json.JSONDecodeError as exc:
            if require_contract:
                validation_errors.append(f"invalid structured status envelope: invalid JSON: {exc}")
            continue
        if not isinstance(payload, dict):
            if require_contract:
                validation_errors.append("invalid structured status envelope: expected a JSON object")
            continue
        if "outcome" not in payload:
            if require_contract:
                validation_errors.append('invalid structured status envelope: missing required top-level key "outcome"')
            continue
        if not set(payload.keys()).issubset(_STRUCTURED_OUTCOME_KEYS):
            unexpected = sorted(set(payload.keys()) - _STRUCTURED_OUTCOME_KEYS)
            unexpected_text = ", ".join(unexpected)
            return None, f"invalid structured status envelope: unexpected top-level keys {unexpected_text}"
        return payload, None
    if require_contract and validation_errors:
        return None, validation_errors[-1]
    return None, None
