from __future__ import annotations

import asyncio
from contextlib import contextmanager, suppress
from datetime import UTC, datetime
import json
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Callable, Optional

from agent.events import EventKind, SessionEvent
from agent.local_environment import LocalExecutionEnvironment
from agent.profiles.anthropic import AnthropicProviderProfile
from agent.profiles.gemini import GeminiProviderProfile
from agent.profiles.openai import OpenAIProviderProfile
from agent.profiles.openai_compatible import OpenAICompatibleProviderProfile
from agent.session import Session
from agent.types import AssistantTurn, SessionConfig, SessionState
from attractor.api.token_usage import (
    TokenUsageBreakdown,
    TokenUsageBucket,
    compute_live_usage_delta,
)
from attractor.api.codergen_contracts import (
    ModeledOutcomeParseResult as _ModeledOutcomeParseResult,
    PlainTextParseResult as _PlainTextParseResult,
    StructuredContractViolation as _StructuredContractViolation,
    build_contract_repair_prompt as _build_contract_repair_prompt,
    coerce_structured_text_outcome as _coerce_structured_text_outcome,
    contract_failure_outcome as _contract_failure_outcome,
    has_response_contract as _has_response_contract,
    validate_write_contract_violation as _validate_write_contract_violation,
    with_write_contract as _with_write_contract,
)
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers.base import CodergenBackend
from spark_common.turn_stream import TurnStreamEvent
from spark.llm_profiles import LlmProfileConfigurationError, get_llm_profile
from unified_llm.client import Client as UnifiedLlmClient
from unified_llm.adapters import OpenAICompatibleAdapter
from unified_llm.models import get_latest_model, get_model_info
from unified_llm.types import Usage
from spark_common.codex_app_client import CodexAppServerClient
from spark_common import codex_app_protocol
from spark_common.runtime_path import resolve_runtime_workspace_path

UNIFIED_AGENT_PROVIDERS = {"openai", "anthropic", "gemini", "openrouter", "litellm", "openai_compatible"}
SUPPORTED_LLM_PROVIDERS = {"codex", *UNIFIED_AGENT_PROVIDERS}
SUPPORTED_LLM_PROVIDER_MESSAGE = (
    "Supported providers: codex, openai, anthropic, gemini, openrouter, litellm, openai_compatible."
)


def _turn_stream_source_payload(event: TurnStreamEvent) -> dict[str, object]:
    payload: dict[str, object] = {}
    for key in (
        "backend",
        "session_id",
        "app_turn_id",
        "item_id",
        "response_id",
        "summary_index",
        "raw_kind",
    ):
        value = getattr(event.source, key)
        if value is not None:
            payload[key] = value
    return payload


def _emit_turn_stream_progress(
    emit_event: Optional[Callable[..., None]],
    *,
    node_id: str,
    event: TurnStreamEvent,
) -> None:
    if emit_event is None:
        return
    if event.kind not in {"content_delta", "content_completed"}:
        return
    if event.channel not in {"assistant", "reasoning", "plan"}:
        return
    content = str(event.content_delta or "")
    if not content:
        return
    emit_event(
        "LLMContent",
        node_id=node_id,
        channel=event.channel,
        content_delta=content,
        status="complete" if event.kind == "content_completed" else "streaming",
        phase=event.phase,
        source=_turn_stream_source_payload(event),
    )


def _emit_session_event_progress(
    emit_event: Optional[Callable[..., None]],
    *,
    node_id: str,
    event: SessionEvent,
) -> None:
    if emit_event is None:
        return
    channel: str
    content: str
    status = "streaming"
    if event.kind == EventKind.ASSISTANT_TEXT_DELTA:
        channel = "assistant"
        content = str(event.data.get("delta", ""))
    elif event.kind == EventKind.ASSISTANT_TEXT_END:
        channel = "assistant"
        content = str(event.data.get("text", ""))
        status = "complete"
    elif event.kind == EventKind.ASSISTANT_REASONING_DELTA:
        channel = "reasoning"
        content = str(event.data.get("delta", ""))
    elif event.kind == EventKind.ASSISTANT_REASONING_END:
        channel = "reasoning"
        content = str(event.data.get("text", "") or "")
        status = "complete"
    else:
        return
    if not content:
        return
    source: dict[str, object] = {
        "backend": "agent_session",
        "raw_kind": str(event.kind),
    }
    response_id = event.data.get("response_id")
    if response_id not in (None, ""):
        source["response_id"] = str(response_id)
    emit_event(
        "LLMContent",
        node_id=node_id,
        channel=channel,
        content_delta=content,
        status=status,
        source=source,
    )


def _is_provider_setup_failure(reason: str) -> bool:
    text = str(reason or "").strip().lower()
    if not text:
        return False
    non_retryable_tokens = (
        "api key",
        "auth",
        "credential",
        "permission",
        "forbidden",
        "unauthorized",
        "configuration",
        "config",
        "not configured",
        "missing",
        "not found on path",
        "unsupported llm_provider",
        "thread/start failed",
    )
    return any(token in text for token in non_retryable_tokens)


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
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        del provider
        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=reason,
                retryable=False if _is_provider_setup_failure(reason) else None,
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
                reasoning_effort=reasoning_effort,
                node_id=node_id,
                emit_event=emit_event,
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
                reasoning_effort=reasoning_effort,
                write_contract=write_contract,
                emit_event=emit_event,
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
        reasoning_effort: Optional[str],
        node_id: str,
        emit_event: Optional[Callable[..., None]] = None,
    ) -> str | None:
        previous_total: TokenUsageBucket | None = None
        saw_usage_update = False

        def handle_turn_event(event: TurnStreamEvent) -> None:
            nonlocal previous_total, saw_usage_update
            _emit_turn_stream_progress(emit_event, node_id=node_id, event=event)
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
            reasoning_effort=reasoning_effort,
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
        reasoning_effort: Optional[str],
        write_contract: ContextWriteContract | None,
        emit_event: Optional[Callable[..., None]] = None,
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
                reasoning_effort=reasoning_effort,
                node_id=node_id,
                emit_event=emit_event,
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


def _normalize_provider(value: Optional[str]) -> str:
    normalized = str(value or "").strip().lower()
    return normalized or "codex"


def _usage_to_bucket(usage: Usage | None) -> TokenUsageBucket:
    if usage is None:
        return TokenUsageBucket()
    return TokenUsageBucket(
        input_tokens=getattr(usage, "input_tokens", 0),
        cached_input_tokens=getattr(usage, "cache_read_tokens", None) or 0,
        output_tokens=getattr(usage, "output_tokens", 0),
        total_tokens=getattr(usage, "total_tokens", 0),
    )


def _breakdown_delta_from(
    current: TokenUsageBreakdown,
    previous: TokenUsageBreakdown | None,
) -> TokenUsageBreakdown:
    delta = TokenUsageBreakdown()
    if current.by_model:
        for model_id, usage in current.by_model.items():
            previous_usage = previous.by_model.get(model_id) if previous is not None else None
            model_delta = usage.delta_from(previous_usage or TokenUsageBucket())
            if model_delta.has_any_usage():
                delta.add_for_model(model_id, model_delta)
        return delta

    current_total = TokenUsageBucket(
        input_tokens=current.input_tokens,
        cached_input_tokens=current.cached_input_tokens,
        output_tokens=current.output_tokens,
        total_tokens=current.total_tokens,
    )
    previous_total = TokenUsageBucket(
        input_tokens=previous.input_tokens,
        cached_input_tokens=previous.cached_input_tokens,
        output_tokens=previous.output_tokens,
        total_tokens=previous.total_tokens,
    ) if previous is not None else TokenUsageBucket()
    aggregate_delta = current_total.delta_from(previous_total)
    if aggregate_delta.has_any_usage():
        delta.add_for_model("unknown", aggregate_delta)
    return delta


def _profile_for_provider(provider: str, model: Optional[str]):
    model_id = str(model or "").strip()
    if provider in {"openrouter", "litellm", "openai_compatible"} and not model_id:
        raise ValueError(f"Provider {provider} requires an explicit model.")
    if not model_id:
        latest = get_latest_model(provider, "tools") or get_latest_model(provider)
        model_id = latest.id if latest is not None else ""
    model_info = get_model_info(model_id) if model_id else None
    supports_streaming = bool(model_info.supports_tools) if model_info is not None else False
    if provider == "openai":
        return OpenAIProviderProfile(model=model_id, supports_streaming=supports_streaming)
    if provider == "anthropic":
        return AnthropicProviderProfile(model=model_id, supports_streaming=supports_streaming)
    if provider == "gemini":
        return GeminiProviderProfile(model=model_id, supports_streaming=supports_streaming)
    if provider in {"openrouter", "litellm"}:
        return OpenAICompatibleProviderProfile(
            id=provider,
            model=model_id,
            display_name="OpenRouter" if provider == "openrouter" else "LiteLLM",
            supports_streaming=supports_streaming,
        )
    if provider == "openai_compatible":
        return OpenAICompatibleProviderProfile(
            id=provider,
            model=model_id,
            display_name="OpenAI Compatible",
            supports_streaming=supports_streaming,
        )
    raise ValueError(f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}")


class UnifiedAgentBackend(CodergenBackend):
    def __init__(
        self,
        working_dir: str,
        emit,
        *,
        provider: str,
        model: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
        client_factory: Callable[[str], UnifiedLlmClient] | None = None,
        config_dir: Path | str | None = None,
    ):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.provider = _normalize_provider(provider)
        self.model = model
        self.reasoning_effort = reasoning_effort
        self.config_dir = Path(config_dir) if config_dir is not None else None
        self._on_usage_update = on_usage_update
        self._client_factory = client_factory or (
            lambda effective_provider: UnifiedLlmClient.from_env(default_provider=effective_provider)
        )
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()

    def _log(self, node_id: str, message: str) -> None:
        if message:
            self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

    def _runtime_failure(self, reason: str) -> Outcome:
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=reason,
            retryable=False if _is_provider_setup_failure(reason) else None,
            failure_kind=FailureKind.RUNTIME,
        )

    def _record_usage(self, *, model: Optional[str], usage: Usage | None) -> None:
        delta = _usage_to_bucket(usage)
        if not delta.has_any_usage():
            return
        normalized_model = str(model or "").strip() or "unified-agent default"
        with self._token_usage_lock:
            self._token_usage_breakdown.add_for_model(normalized_model, delta)
            snapshot = self._token_usage_breakdown.copy()
        if self._on_usage_update is not None:
            self._on_usage_update(snapshot)

    def _handle_event(
        self,
        node_id: str,
        event: SessionEvent,
        emit_event: Optional[Callable[..., None]],
    ) -> None:
        _emit_session_event_progress(
            emit_event,
            node_id=node_id,
            event=event,
        )
        if event.kind == EventKind.ASSISTANT_TEXT_DELTA:
            delta = str(event.data.get("delta", ""))
            if delta.strip():
                self._log(node_id, delta)
            return
        if event.kind == EventKind.TOOL_CALL_START:
            tool_name = str(event.data.get("tool_name", "tool"))
            self._log(node_id, f"tool started: {tool_name}")
            return
        if event.kind == EventKind.TOOL_CALL_END:
            tool_name = str(event.data.get("tool_name", "tool"))
            suffix = "failed" if event.data.get("error") is not None else "completed"
            self._log(node_id, f"tool {suffix}: {tool_name}")
            return
        if event.kind == EventKind.ERROR:
            self._log(node_id, str(event.data.get("error", "")))

    async def _submit_and_capture(
        self,
        session: Session,
        node_id: str,
        prompt: str,
        emit_event: Optional[Callable[..., None]],
    ) -> str:
        task = asyncio.create_task(session.process_input(prompt))
        try:
            while True:
                if task.done() and session.event_queue.empty():
                    break
                try:
                    event = await asyncio.wait_for(session.event_queue.get(), timeout=0.05)
                except asyncio.TimeoutError:
                    continue
                self._handle_event(node_id, event, emit_event)
            await task
        except BaseException:
            if not task.done():
                task.cancel()
                with suppress(asyncio.CancelledError):
                    await task
            raise
        if session.state == SessionState.AWAITING_INPUT:
            raise RuntimeError("unified-agent codergen requested interactive input; this is not supported for v1")
        for turn in reversed(session.history):
            if isinstance(turn, AssistantTurn):
                self._record_usage(model=session.provider_profile.model, usage=turn.usage)
                return turn.text
        return ""

    async def _run_session(
        self,
        node_id: str,
        prompt: str,
        *,
        provider: str,
        model: Optional[str],
        reasoning_effort: Optional[str],
        response_contract: str,
        contract_repair_attempts: int,
        write_contract: ContextWriteContract | None,
        llm_profile: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
    ) -> str | Outcome:
        if llm_profile:
            if self.config_dir is None:
                raise LlmProfileConfigurationError("LLM profile config directory is unavailable.")
            configured_profile = get_llm_profile(self.config_dir, llm_profile)
            profile = OpenAICompatibleProviderProfile(
                id=configured_profile.id,
                model=str(model or "").strip(),
                display_name=configured_profile.label or configured_profile.id,
                supports_streaming=True,
            )
            client = UnifiedLlmClient(
                providers={configured_profile.id: configured_profile.build_adapter()},
                default_provider=configured_profile.id,
            )
        else:
            profile = _profile_for_provider(provider, model)
            client = self._client_factory(provider)
        session = Session(
            provider_profile=profile,
            execution_environment=LocalExecutionEnvironment(working_dir=self.working_dir),
            client=client,
            config=SessionConfig(reasoning_effort=reasoning_effort),
        )
        try:
            response_text = await self._submit_and_capture(session, node_id, prompt, emit_event)
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
                self._log(
                    node_id,
                    f"response contract violation for {node_id}; requesting corrected final answer "
                    f"(attempt {attempt}/{contract_repair_attempts}): {current_violation.reason}",
                )
                repair_text = await self._submit_and_capture(
                    session,
                    node_id,
                    _build_contract_repair_prompt(current_violation),
                    emit_event,
                )
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
                if isinstance(repaired, Outcome):
                    return repaired
                current_violation = _with_write_contract(repaired, write_contract)
            return _contract_failure_outcome(current_violation)
        finally:
            await session.close()
            close_client = getattr(client, "close", None)
            if callable(close_client):
                result = close_client()
                if asyncio.iscoroutine(result):
                    await result

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
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        llm_profile: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        del context
        effective_provider = _normalize_provider(provider or self.provider)
        if not llm_profile and effective_provider not in UNIFIED_AGENT_PROVIDERS:
            return self._runtime_failure(
                f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}"
            )
        try:
            session_coro = self._run_session(
                node_id,
                prompt,
                provider=effective_provider,
                model=model or self.model,
                reasoning_effort=reasoning_effort or self.reasoning_effort,
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                llm_profile=llm_profile,
                emit_event=emit_event,
                write_contract=write_contract,
            )
            if timeout is not None and timeout > 0:
                session_coro = asyncio.wait_for(session_coro, timeout=timeout)
            return asyncio.run(
                session_coro
            )
        except asyncio.TimeoutError:
            timeout_text = f"{timeout:g}" if timeout is not None else ""
            return self._runtime_failure(f"unified-agent backend timed out after {timeout_text}s")
        except RuntimeError as exc:
            return self._runtime_failure(str(exc))
        except Exception as exc:
            return self._runtime_failure(str(exc) or exc.__class__.__name__)


class ProviderRouterBackend(CodergenBackend):
    def __init__(
        self,
        working_dir: str,
        emit,
        *,
        model: Optional[str] = None,
        on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
        config_dir: Path | str | None = None,
    ):
        self.working_dir = working_dir
        self.emit = emit
        self.model = model
        self.provider = "codex"
        self.config_dir = Path(config_dir) if config_dir is not None else None
        self._on_usage_update = on_usage_update
        self._token_usage_lock = threading.Lock()
        self._token_usage_breakdown = TokenUsageBreakdown()
        self._source_usage_snapshots: dict[object, TokenUsageBreakdown] = {}
        self._codex_usage_source = object()
        self._codex = CodexAppServerBackend(
            working_dir,
            emit,
            model=model,
            on_usage_update=lambda snapshot: self._record_source_usage(
                self._codex_usage_source,
                snapshot,
            ),
        )

    def bind_stage_raw_rpc_log(self, node_id: str, logs_root: str | Path | None):
        return self._codex.bind_stage_raw_rpc_log(node_id, logs_root)

    def _record_source_usage(self, source_key: object, source_snapshot: TokenUsageBreakdown) -> None:
        with self._token_usage_lock:
            previous_snapshot = self._source_usage_snapshots.get(source_key)
            source_delta = _breakdown_delta_from(source_snapshot, previous_snapshot)
            self._source_usage_snapshots[source_key] = source_snapshot.copy()
            if not source_delta.has_any_usage():
                return
            for model_id, usage in source_delta.by_model.items():
                self._token_usage_breakdown.add_for_model(model_id, usage)
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
        provider: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        llm_profile: Optional[str] = None,
        emit_event: Optional[Callable[..., None]] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        effective_provider = _normalize_provider(provider)
        if effective_provider == "codex" and not llm_profile:
            return self._codex.run(
                node_id,
                prompt,
                context,
                response_contract=response_contract,
                contract_repair_attempts=contract_repair_attempts,
                timeout=timeout,
                model=model,
                reasoning_effort=reasoning_effort,
                emit_event=emit_event,
                write_contract=write_contract,
            )
        if llm_profile or effective_provider in UNIFIED_AGENT_PROVIDERS:
            usage_source = object()
            backend = UnifiedAgentBackend(
                self.working_dir,
                self.emit,
                provider=effective_provider,
                model=model or self.model,
                reasoning_effort=reasoning_effort,
                on_usage_update=lambda snapshot: self._record_source_usage(usage_source, snapshot),
                config_dir=self.config_dir,
            )
            kwargs = {
                "response_contract": response_contract,
                "contract_repair_attempts": contract_repair_attempts,
                "timeout": timeout,
                "model": model,
                "provider": effective_provider,
                "reasoning_effort": reasoning_effort,
                "emit_event": emit_event,
                "write_contract": write_contract,
            }
            if llm_profile:
                kwargs["llm_profile"] = llm_profile
            return backend.run(node_id, prompt, context, **kwargs)
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=(
                f"Unsupported llm_provider. {SUPPORTED_LLM_PROVIDER_MESSAGE}"
            ),
            retryable=False,
            failure_kind=FailureKind.RUNTIME,
        )


def build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
    on_usage_update: Optional[Callable[[TokenUsageBreakdown], None]] = None,
    config_dir: Path | str | None = None,
) -> CodergenBackend:
    normalized = backend_name.strip().lower()
    if normalized in {"", "provider-router"}:
        return ProviderRouterBackend(
            working_dir,
            emit,
            model=model,
            on_usage_update=on_usage_update,
            config_dir=config_dir,
        )
    if normalized == "codex-app-server":
        return CodexAppServerBackend(working_dir, emit, model=model, on_usage_update=on_usage_update)
    raise ValueError(
        "Unsupported backend. Supported backends: provider-router, codex-app-server."
    )
