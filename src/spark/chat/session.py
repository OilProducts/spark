from __future__ import annotations

import asyncio
import copy
from dataclasses import dataclass, field
from pathlib import Path
import subprocess
import threading
import tomllib
import uuid
from typing import Any, Callable, Optional

from agent.events import EventKind, SessionEvent
from agent.local_environment import LocalExecutionEnvironment
from agent.profiles.anthropic import AnthropicProviderProfile
from agent.profiles.gemini import GeminiProviderProfile
from agent.profiles.openai import OpenAIProviderProfile
from agent.session import Session
from agent.types import AssistantTurn, SessionConfig, SessionState, UserTurn
from spark.workspace.conversations.models import (
    ChatTurnResult,
    RequestUserInputOption,
    RequestUserInputQuestion,
    RequestUserInputRecord,
    ToolCallRecord,
)
from spark_common.turn_stream import TurnStreamEvent, TurnStreamSource
from spark.workspace.conversations.utils import (
    as_non_empty_string,
    normalize_project_path_value,
)
from spark_common.codex_app_client import (
    APP_SERVER_REQUEST_TIMEOUT_SECONDS,
    CodexAppServerClient,
)
from spark_common import codex_app_protocol
from spark_common.codex_runtime import build_codex_runtime_environment
from spark_common.runtime_path import resolve_runtime_workspace_path
from unified_llm.client import Client as UnifiedLlmClient
from unified_llm.models import get_latest_model, get_model_info


CHAT_TURN_IDLE_TIMEOUT_SECONDS = codex_app_protocol.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS


def _normalize_provider(value: str | None) -> str:
    normalized = str(value or "").strip().lower()
    return normalized or "codex"


def _profile_for_provider(provider: str, model: str | None):
    model_id = as_non_empty_string(model)
    if model_id is None:
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
    raise ValueError("Provider must be blank or one of: codex, openai, anthropic, gemini.")


def _tool_record_for_session_event(event: SessionEvent, *, status: str) -> ToolCallRecord:
    tool_name = str(event.data.get("tool_name") or "tool")
    tool_call_id = as_non_empty_string(event.data.get("tool_call_id")) or f"tool-{uuid.uuid4().hex}"
    output = event.data.get("output")
    error = event.data.get("error")
    return ToolCallRecord(
        id=tool_call_id,
        kind="dynamic_tool",
        status=status,
        title=tool_name,
        output=str(error if error is not None else output) if (error is not None or output is not None) else None,
    )


def _normalize_tool_call_status(value: Any) -> str:
    normalized = str(value or "").strip().lower()
    if normalized in {"inprogress", "running"}:
        return "running"
    if normalized in {"failed", "error"}:
        return "failed"
    return "completed"


def _tool_call_from_item(item: dict[str, Any]) -> Optional[ToolCallRecord]:
    item_type = str(item.get("type") or "").strip()
    item_id = as_non_empty_string(item.get("id")) or f"tool-{uuid.uuid4().hex}"
    if item_type == "commandExecution":
        command = codex_app_protocol.extract_command_text(item)
        raw_output = item.get("aggregatedOutput")
        if raw_output is None:
            raw_output = item.get("aggregated_output")
        output = str(raw_output) if raw_output is not None and str(raw_output) else None
        return ToolCallRecord(
            id=item_id,
            kind="command_execution",
            status=_normalize_tool_call_status(item.get("status")),
            title="Run command",
            command=command,
            output=output,
        )
    if item_type == "fileChange":
        return ToolCallRecord(
            id=item_id,
            kind="file_change",
            status=_normalize_tool_call_status(item.get("status")),
            title="Apply file changes",
            file_paths=codex_app_protocol.extract_file_paths(item),
        )
    return None


def _request_user_input_question_type(question: dict[str, Any]) -> str:
    options = question.get("options")
    if isinstance(options, list) and len(options) > 0:
        return "MULTIPLE_CHOICE"
    return "FREEFORM"


def _request_user_input_record_from_payload(payload: dict[str, Any]) -> Optional[RequestUserInputRecord]:
    request_id = as_non_empty_string(payload.get("itemId"))
    raw_questions = payload.get("questions")
    if not request_id or not isinstance(raw_questions, list) or len(raw_questions) == 0:
        return None
    questions: list[RequestUserInputQuestion] = []
    for index, entry in enumerate(raw_questions):
        if not isinstance(entry, dict):
            continue
        question_id = as_non_empty_string(entry.get("id")) or f"question-{index + 1}"
        prompt = as_non_empty_string(entry.get("question"))
        if not prompt:
            continue
        raw_options = entry.get("options")
        options = [
            RequestUserInputOption(
                label=str(option.get("label", "")),
                description=str(option.get("description")) if option.get("description") is not None else None,
            )
            for option in raw_options
            if isinstance(option, dict) and as_non_empty_string(option.get("label"))
        ] if isinstance(raw_options, list) else []
        questions.append(
            RequestUserInputQuestion(
                id=question_id,
                header=as_non_empty_string(entry.get("header")) or f"Question {index + 1}",
                question=prompt,
                question_type=_request_user_input_question_type(entry),
                options=options,
                allow_other=bool(entry.get("isOther")),
                is_secret=bool(entry.get("isSecret")),
            )
        )
    if len(questions) == 0:
        return None
    return RequestUserInputRecord(
        request_id=request_id,
        status="pending",
        questions=questions,
    )


def _request_user_input_response_payload(answers: dict[str, str]) -> dict[str, Any]:
    return {
        "answers": {
            str(question_id): {"answers": [str(answer)]}
            for question_id, answer in answers.items()
            if str(answer).strip()
        }
    }


def _token_usage_payload_from_unified_usage(usage: Any) -> Optional[dict[str, Any]]:
    if usage is None:
        return None
    if isinstance(usage, dict):
        input_tokens = int(usage.get("input_tokens", 0) or 0)
        cached_input_tokens = int(usage.get("cache_read_tokens", 0) or 0)
        output_tokens = int(usage.get("output_tokens", 0) or 0)
        total_tokens = int(usage.get("total_tokens", 0) or 0)
        if total_tokens <= 0:
            total_tokens = input_tokens + output_tokens
        if max(input_tokens, cached_input_tokens, output_tokens, total_tokens) <= 0:
            return None
        return {
            "total": {
                "inputTokens": max(0, input_tokens),
                "cachedInputTokens": max(0, min(input_tokens, cached_input_tokens)),
                "outputTokens": max(0, output_tokens),
                "totalTokens": max(0, total_tokens),
            }
        }
    input_tokens = int(getattr(usage, "input_tokens", 0) or 0)
    cached_input_tokens = int(getattr(usage, "cache_read_tokens", 0) or 0)
    output_tokens = int(getattr(usage, "output_tokens", 0) or 0)
    total_tokens = int(getattr(usage, "total_tokens", 0) or 0)
    if total_tokens <= 0:
        total_tokens = input_tokens + output_tokens
    if max(input_tokens, cached_input_tokens, output_tokens, total_tokens) <= 0:
        return None
    return {
        "total": {
            "inputTokens": max(0, input_tokens),
            "cachedInputTokens": max(0, min(input_tokens, cached_input_tokens)),
            "outputTokens": max(0, output_tokens),
            "totalTokens": max(0, total_tokens),
        }
    }


def _agent_history_from_persisted_turns(turns: list[Any]) -> list[UserTurn | AssistantTurn]:
    history: list[UserTurn | AssistantTurn] = []
    for turn in turns:
        role = str(getattr(turn, "role", "") or "").strip().lower()
        status = str(getattr(turn, "status", "") or "").strip().lower()
        kind = str(getattr(turn, "kind", "message") or "message").strip().lower()
        content = str(getattr(turn, "content", "") or "")
        if kind != "message" or status != "complete" or not content:
            continue
        if role == "user":
            history.append(UserTurn(content))
        elif role == "assistant":
            history.append(AssistantTurn(content))
    return history


@dataclass
class _PendingUserInputRequest:
    request_id: str
    question_ids: tuple[str, ...]
    condition: threading.Condition = field(default_factory=threading.Condition)
    answers: Optional[dict[str, str]] = None

    def wait_for_answers(self) -> dict[str, str]:
        with self.condition:
            while self.answers is None:
                self.condition.wait()
            return dict(self.answers)

    def submit(self, answers: dict[str, str]) -> None:
        with self.condition:
            self.answers = dict(answers)
            self.condition.notify_all()


class UnifiedAgentChatSession:
    def __init__(
        self,
        working_dir: str,
        *,
        provider: str,
        model: Optional[str] = None,
        persisted_history: list[Any] | None = None,
        client_factory: Callable[[str], UnifiedLlmClient] | None = None,
    ) -> None:
        self.requested_working_dir = normalize_project_path_value(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.provider = _normalize_provider(provider)
        self.model = as_non_empty_string(model)
        self._persisted_history = list(persisted_history or [])
        self._client_factory = client_factory or (
            lambda effective_provider: UnifiedLlmClient.from_env(default_provider=effective_provider)
        )
        self._session: Session | None = None
        self._client: UnifiedLlmClient | None = None
        self._runner: asyncio.Runner | None = None
        self._lock = threading.Lock()

    def _close_session_and_client_unlocked(self, *, close_runner: bool) -> None:
        session = self._session
        self._session = None
        client = self._client
        self._client = None
        runner = self._runner
        if close_runner:
            self._runner = None
        if session is not None:
            try:
                if runner is not None:
                    runner.run(session.close())
                else:
                    asyncio.run(session.close())
            except RuntimeError:
                pass
        if client is not None:
            close_client = getattr(client, "close", None)
            if callable(close_client):
                try:
                    result = close_client()
                    if asyncio.iscoroutine(result):
                        if runner is not None:
                            runner.run(result)
                        else:
                            asyncio.run(result)
                except RuntimeError:
                    pass
        if close_runner and runner is not None:
            runner.close()

    def close(self) -> None:
        with self._lock:
            self._close_session_and_client_unlocked(close_runner=True)

    def _replace_model_unlocked(self, model: Optional[str]) -> None:
        next_model = as_non_empty_string(model)
        if next_model == self.model:
            return
        self._close_session_and_client_unlocked(close_runner=False)
        self.model = next_model

    def _run_async(self, coro):
        if self._runner is None:
            self._runner = asyncio.Runner()
        return self._runner.run(coro)

    def _build_session(self, reasoning_effort: Optional[str]) -> Session:
        profile = _profile_for_provider(self.provider, self.model)
        client = self._client_factory(self.provider)
        self._client = client
        session = Session(
            provider_profile=profile,
            execution_environment=LocalExecutionEnvironment(working_dir=self.working_dir),
            client=client,
            config=SessionConfig(reasoning_effort=reasoning_effort),
        )
        session.history.extend(_agent_history_from_persisted_turns(self._persisted_history))
        return session

    def _emit_live_event(
        self,
        callback: Optional[Callable[[TurnStreamEvent], None]],
        event: TurnStreamEvent,
    ) -> None:
        if callback is not None:
            callback(event)

    def _forward_session_event(
        self,
        event: SessionEvent,
        *,
        on_event: Optional[Callable[[TurnStreamEvent], None]],
    ) -> None:
        if event.kind == EventKind.ASSISTANT_TEXT_DELTA:
            delta = str(event.data.get("delta", ""))
            if delta:
                self._emit_live_event(
                    on_event,
                    TurnStreamEvent(
                        kind="content_delta",
                        channel="assistant",
                        content_delta=delta,
                        source=TurnStreamSource(
                            backend="agent_session",
                            response_id=as_non_empty_string(event.data.get("response_id")),
                            raw_kind=str(event.kind),
                        ),
                    ),
                )
            return
        if event.kind == EventKind.ASSISTANT_TEXT_END:
            text = str(event.data.get("text", ""))
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="content_completed",
                    channel="assistant",
                    content_delta=text,
                    source=TurnStreamSource(
                        backend="agent_session",
                        response_id=as_non_empty_string(event.data.get("response_id")),
                        raw_kind=str(event.kind),
                    ),
                ),
            )
            return
        if event.kind == EventKind.ASSISTANT_REASONING_DELTA:
            delta = str(event.data.get("delta", ""))
            if delta:
                self._emit_live_event(
                    on_event,
                    TurnStreamEvent(
                        kind="content_delta",
                        channel="reasoning",
                        content_delta=delta,
                        source=TurnStreamSource(
                            backend="agent_session",
                            response_id=as_non_empty_string(event.data.get("response_id")),
                            raw_kind=str(event.kind),
                        ),
                    ),
                )
            return
        if event.kind == EventKind.ASSISTANT_REASONING_END:
            text = str(event.data.get("text", "") or "")
            if text:
                self._emit_live_event(
                    on_event,
                    TurnStreamEvent(
                        kind="content_completed",
                        channel="reasoning",
                        content_delta=text,
                        source=TurnStreamSource(
                            backend="agent_session",
                            response_id=as_non_empty_string(event.data.get("response_id")),
                            raw_kind=str(event.kind),
                        ),
                    ),
                )
            return
        if event.kind == EventKind.MODEL_USAGE_UPDATE:
            token_usage = _token_usage_payload_from_unified_usage(event.data.get("usage"))
            if token_usage is not None:
                self._emit_live_event(
                    on_event,
                    TurnStreamEvent(
                        kind="token_usage_updated",
                        token_usage=token_usage,
                        source=TurnStreamSource(backend="agent_session", raw_kind=str(event.kind)),
                    ),
                )
            return
        if event.kind in {
            EventKind.MODEL_TOOL_CALL_START,
            EventKind.MODEL_TOOL_CALL_DELTA,
            EventKind.MODEL_TOOL_CALL_END,
        }:
            return
        if event.kind == EventKind.TOOL_CALL_START:
            tool_call = _tool_record_for_session_event(event, status="running")
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_started",
                    tool_call=tool_call,
                    source=TurnStreamSource(backend="agent_session", item_id=tool_call.id, raw_kind=str(event.kind)),
                ),
            )
            return
        if event.kind == EventKind.TOOL_CALL_END:
            status = "failed" if event.data.get("error") is not None else "completed"
            tool_call = _tool_record_for_session_event(event, status=status)
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_failed" if status == "failed" else "tool_call_completed",
                    tool_call=tool_call,
                    source=TurnStreamSource(backend="agent_session", item_id=tool_call.id, raw_kind=str(event.kind)),
                ),
            )
            return
        if event.kind == EventKind.ERROR:
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="error",
                    message=str(event.data.get("error", "")),
                    error=str(event.data.get("error", "")),
                    source=TurnStreamSource(backend="agent_session", raw_kind=str(event.kind)),
                ),
            )

    async def _submit_and_capture(
        self,
        session: Session,
        prompt: str,
        *,
        on_event: Optional[Callable[[TurnStreamEvent], None]],
    ) -> tuple[str, Optional[dict[str, Any]]]:
        task = asyncio.create_task(session.process_input(prompt))
        while True:
            if task.done() and session.event_queue.empty():
                break
            try:
                event = await asyncio.wait_for(session.event_queue.get(), timeout=0.05)
            except asyncio.TimeoutError:
                continue
            self._forward_session_event(event, on_event=on_event)
        await task
        if session.state == SessionState.AWAITING_INPUT:
            raise RuntimeError("unified-agent project chat requested interactive input; this is not supported")
        for turn in reversed(session.history):
            if isinstance(turn, AssistantTurn):
                return turn.text, _token_usage_payload_from_unified_usage(getattr(turn, "usage", None))
        return "", None

    def turn(
        self,
        prompt: str,
        model: Optional[str],
        *,
        chat_mode: str = "chat",
        reasoning_effort: Optional[str] = None,
        on_event: Optional[Callable[[TurnStreamEvent], None]] = None,
    ) -> ChatTurnResult:
        del chat_mode
        with self._lock:
            if model is not None:
                self._replace_model_unlocked(model)
            if self._session is None:
                self._session = self._build_session(reasoning_effort)
            else:
                self._session.config.reasoning_effort = reasoning_effort
            message, token_usage = self._run_async(
                self._submit_and_capture(
                    self._session,
                    prompt,
                    on_event=on_event,
                )
            )
            return ChatTurnResult(assistant_message=message, token_usage=token_usage)


class CodexAppServerChatSession:
    def __init__(
        self,
        working_dir: str,
        *,
        persisted_thread_id: Optional[str] = None,
        persisted_model: Optional[str] = None,
        on_thread_id_updated: Optional[Callable[[str], None]] = None,
        on_model_updated: Optional[Callable[[str], None]] = None,
    ) -> None:
        self.requested_working_dir = normalize_project_path_value(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self._thread_id: Optional[str] = persisted_thread_id
        self._model: Optional[str] = as_non_empty_string(persisted_model)
        self._thread_initialized = False
        self._on_thread_id_updated = on_thread_id_updated
        self._on_model_updated = on_model_updated
        self._client = CodexAppServerClient(
            self.working_dir,
            requested_working_dir=self.requested_working_dir or self.working_dir,
            request_timeout_seconds=APP_SERVER_REQUEST_TIMEOUT_SECONDS,
        )
        self._lock = threading.Lock()
        self._pending_user_input_lock = threading.Lock()
        self._pending_user_input_by_request_id: dict[str, _PendingUserInputRequest] = {}
        self._pending_user_input_request_id_by_question_id: dict[str, str] = {}

    def _close_unlocked(self) -> None:
        self._client.close()
        self._thread_initialized = False

    def close(self) -> None:
        with self._lock:
            self._close_unlocked()

    def set_raw_rpc_logger(self, callback: Optional[Callable[[str, str], None]]) -> None:
        self._client.set_raw_rpc_logger(callback)

    def clear_raw_rpc_logger(self) -> None:
        self._client.clear_raw_rpc_logger()

    def _register_pending_user_input(self, request: RequestUserInputRecord) -> _PendingUserInputRequest:
        pending = _PendingUserInputRequest(
            request_id=request.request_id,
            question_ids=tuple(question.id for question in request.questions),
        )
        with self._pending_user_input_lock:
            self._pending_user_input_by_request_id[request.request_id] = pending
            for question_id in pending.question_ids:
                self._pending_user_input_request_id_by_question_id[question_id] = request.request_id
        return pending

    def _clear_pending_user_input(self, request_id: str) -> None:
        with self._pending_user_input_lock:
            pending = self._pending_user_input_by_request_id.pop(request_id, None)
            if pending is None:
                return
            for question_id in pending.question_ids:
                current_request_id = self._pending_user_input_request_id_by_question_id.get(question_id)
                if current_request_id == request_id:
                    self._pending_user_input_request_id_by_question_id.pop(question_id, None)

    def submit_request_user_input_answers(self, request_or_question_id: str, answers: dict[str, str]) -> bool:
        normalized_lookup_id = as_non_empty_string(request_or_question_id)
        if not normalized_lookup_id:
            return False
        normalized_answers = {
            str(key): str(value).strip()
            for key, value in answers.items()
            if str(value).strip()
        }
        if len(normalized_answers) == 0:
            return False
        with self._pending_user_input_lock:
            request_id = self._pending_user_input_request_id_by_question_id.get(normalized_lookup_id, normalized_lookup_id)
            pending = self._pending_user_input_by_request_id.get(request_id)
        if pending is None:
            return False
        pending.submit(normalized_answers)
        return True

    def has_pending_request_user_input(self, request_or_question_id: str) -> bool:
        normalized_lookup_id = as_non_empty_string(request_or_question_id)
        if not normalized_lookup_id:
            return False
        with self._pending_user_input_lock:
            request_id = self._pending_user_input_request_id_by_question_id.get(normalized_lookup_id, normalized_lookup_id)
            return request_id in self._pending_user_input_by_request_id

    def _ensure_process(self) -> None:
        previous_proc = self._client.proc
        self._client.ensure_process(popen_factory=subprocess.Popen)
        if self._client.proc is not previous_proc:
            self._thread_initialized = False

    def _set_thread_id(self, thread_id: str) -> None:
        normalized_thread_id = as_non_empty_string(thread_id)
        if not normalized_thread_id:
            return
        self._thread_id = normalized_thread_id
        if self._on_thread_id_updated is not None:
            self._on_thread_id_updated(normalized_thread_id)

    def _set_model(self, model: Optional[str]) -> Optional[str]:
        normalized_model = as_non_empty_string(model)
        if not normalized_model:
            return None
        if normalized_model == self._model:
            return normalized_model
        self._model = normalized_model
        if self._on_model_updated is not None:
            self._on_model_updated(normalized_model)
        return normalized_model

    def _configured_runtime_model(self) -> Optional[str]:
        env = build_codex_runtime_environment()
        codex_home_value = str(env.get("CODEX_HOME", "")).strip()
        if not codex_home_value:
            return None
        codex_home = Path(codex_home_value).expanduser()
        config_path = codex_home / "config.toml"
        try:
            payload = tomllib.loads(config_path.read_text(encoding="utf-8"))
        except (FileNotFoundError, OSError, tomllib.TOMLDecodeError):
            return None
        return as_non_empty_string(payload.get("model"))

    def _resolve_turn_model(self, model: Optional[str]) -> str:
        explicit_model = self._set_model(model)
        if explicit_model is not None:
            return explicit_model
        if self._model is not None:
            return self._model
        configured_model = self._set_model(self._configured_runtime_model())
        if configured_model is not None:
            return configured_model
        default_model = self._set_model(self._client.default_model())
        if default_model is not None:
            return default_model
        raise RuntimeError("codex app-server model is unavailable for the chat session")

    def _ensure_thread(self, model: Optional[str]) -> None:
        if self._thread_initialized and self._thread_id:
            return
        if self._thread_id:
            resumed_thread_id = self._client.resume_thread(
                self._thread_id,
                model=model,
                cwd=self.working_dir,
                approval_policy="never",
            )
        else:
            resumed_thread_id = None
        if resumed_thread_id:
            self._set_thread_id(resumed_thread_id)
            self._thread_initialized = True
            return
        started_thread_id = self._client.start_thread(
            model=model,
            cwd=self.working_dir,
            approval_policy="never",
            ephemeral=False,
        )
        self._set_thread_id(started_thread_id)
        self._thread_initialized = True

    def _emit_live_event(
        self,
        callback: Optional[Callable[[TurnStreamEvent], None]],
        event: TurnStreamEvent,
    ) -> None:
        if callback is None:
            return
        callback(event)

    def _handle_request_user_input_server_request(
        self,
        message: dict[str, Any],
        *,
        on_event: Optional[Callable[[TurnStreamEvent], None]],
        current_app_turn_id: Optional[str],
    ) -> dict[str, Any]:
        params = message.get("params") or {}
        request = _request_user_input_record_from_payload(params) if isinstance(params, dict) else None
        request_id = message.get("id")
        if request is None or request_id is None:
            self._client.send_response(
                request_id,
                error={"code": -32000, "message": "Malformed request_user_input request."},
            )
            return {
                "jsonrpc": message.get("jsonrpc", "2.0"),
                "method": "item/tool/requestUserInput/handled",
                "params": params if isinstance(params, dict) else {},
            }
        self._emit_live_event(
            on_event,
            TurnStreamEvent(
                kind="request_user_input_requested",
                source=TurnStreamSource(
                    backend="codex_app_server",
                    app_turn_id=current_app_turn_id,
                    item_id=request.request_id,
                    raw_kind="request_user_input_requested",
                ),
                request_user_input=request,
            ),
        )
        pending_request = self._register_pending_user_input(request)
        try:
            answers = pending_request.wait_for_answers()
        finally:
            self._clear_pending_user_input(request.request_id)
        self._client.send_response(request_id, _request_user_input_response_payload(answers))
        return {
            "jsonrpc": message.get("jsonrpc", "2.0"),
            "method": "item/tool/requestUserInput/handled",
            "params": params,
        }

    def _forward_normalized_turn_event(
        self,
        normalized_event: TurnStreamEvent,
        *,
        on_event: Optional[Callable[[TurnStreamEvent], None]],
        tool_calls_by_id: dict[str, ToolCallRecord],
        current_app_turn_id: Optional[str],
    ) -> None:
        source = TurnStreamSource(
            backend=normalized_event.source.backend,
            session_id=normalized_event.source.session_id,
            app_turn_id=current_app_turn_id or normalized_event.source.app_turn_id,
            item_id=normalized_event.source.item_id,
            response_id=normalized_event.source.response_id,
            summary_index=normalized_event.source.summary_index,
            raw_kind=normalized_event.source.raw_kind,
        )
        if normalized_event.kind == "tool_call_started" and normalized_event.source.raw_kind == "command_approval_requested":
            payload = normalized_event.tool_call or {}
            item_id = normalized_event.source.item_id or f"tool-{uuid.uuid4().hex}"
            tool_call = ToolCallRecord(
                id=item_id,
                kind="command_execution",
                status="running",
                title="Run command",
                command=normalized_event.content_delta or codex_app_protocol.extract_command_text(payload),
            )
            tool_calls_by_id[item_id] = tool_call
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_started",
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "tool_call_started" and normalized_event.source.raw_kind == "file_change_approval_requested":
            payload = normalized_event.tool_call or {}
            item_id = normalized_event.source.item_id or f"tool-{uuid.uuid4().hex}"
            tool_call = ToolCallRecord(
                id=item_id,
                kind="file_change",
                status="running",
                title="Apply file changes",
                file_paths=codex_app_protocol.extract_file_paths(payload),
            )
            tool_calls_by_id[item_id] = tool_call
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_started",
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "content_delta" and normalized_event.content_delta:
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="content_delta",
                    channel=normalized_event.channel,
                    content_delta=normalized_event.content_delta,
                    source=source,
                    phase=normalized_event.phase,
                ),
            )
            return
        if normalized_event.kind == "content_completed" and normalized_event.content_delta:
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="content_completed",
                    channel=normalized_event.channel,
                    content_delta=normalized_event.content_delta,
                    message="Plan item completed." if normalized_event.channel == "plan" else "Assistant message completed.",
                    source=source,
                    phase=normalized_event.phase,
                ),
            )
            return
        if normalized_event.kind == "context_compaction_started":
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="context_compaction_started",
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "context_compaction_completed":
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="context_compaction_completed",
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "request_user_input_requested" and isinstance(normalized_event.request_user_input, dict):
            request = _request_user_input_record_from_payload(normalized_event.request_user_input)
            if request is None:
                return
            source.item_id = request.request_id
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="request_user_input_requested",
                    source=source,
                    request_user_input=request,
                ),
            )
            return
        if normalized_event.kind == "tool_call_started" and isinstance(normalized_event.tool_call, dict):
            tool_call = _tool_call_from_item(normalized_event.tool_call)
            if tool_call is None:
                return
            if normalized_event.source.item_id:
                tool_calls_by_id[normalized_event.source.item_id] = tool_call
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_started",
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "tool_call_completed" and isinstance(normalized_event.tool_call, dict):
            tool_call = _tool_call_from_item(normalized_event.tool_call)
            if tool_call is None:
                return
            if normalized_event.source.item_id:
                tool_calls_by_id[normalized_event.source.item_id] = tool_call
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_failed" if tool_call.status == "failed" else "tool_call_completed",
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    source=source,
                ),
            )
            return
        if (
            normalized_event.kind == "tool_call_updated"
            and normalized_event.source.raw_kind == "command_output_delta"
            and normalized_event.content_delta
        ):
            tool_call = tool_calls_by_id.get(normalized_event.source.item_id or "")
            if tool_call is None:
                return
            tool_call.output = codex_app_protocol.append_tool_output(tool_call.output, normalized_event.content_delta)
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="tool_call_updated",
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    source=source,
                ),
            )
            return
        if normalized_event.kind == "token_usage_updated" and normalized_event.token_usage is not None:
            self._emit_live_event(
                on_event,
                TurnStreamEvent(
                    kind="token_usage_updated",
                    source=source,
                    token_usage=copy.deepcopy(normalized_event.token_usage),
                ),
            )

    def turn(
        self,
        prompt: str,
        model: Optional[str],
        *,
        chat_mode: str = "chat",
        reasoning_effort: Optional[str] = None,
        on_event: Optional[Callable[[TurnStreamEvent], None]] = None,
    ) -> ChatTurnResult:
        with self._lock:
            self._ensure_process()
            effective_model = self._resolve_turn_model(model)
            self._ensure_thread(effective_model)
            tool_calls_by_id: dict[str, ToolCallRecord] = {}
            current_app_turn_id: Optional[str] = None

            def _handle_turn_started(turn_id: str) -> None:
                nonlocal current_app_turn_id
                current_app_turn_id = turn_id

            def _handle_server_request(message: dict[str, Any]) -> dict[str, Any]:
                method = message.get("method")
                if method != "item/tool/requestUserInput":
                    return self._client._handle_server_request(message)
                return self._handle_request_user_input_server_request(
                    message,
                    on_event=on_event,
                    current_app_turn_id=current_app_turn_id,
                )

            def _handle_normalized_event(normalized_event: TurnStreamEvent) -> None:
                self._forward_normalized_turn_event(
                    normalized_event,
                    on_event=on_event,
                    tool_calls_by_id=tool_calls_by_id,
                    current_app_turn_id=current_app_turn_id,
                )

            try:
                result = self._client.run_turn(
                    thread_id=self._thread_id or "",
                    prompt=prompt,
                    model=effective_model,
                    reasoning_effort=reasoning_effort,
                    chat_mode=chat_mode,
                    cwd=self.working_dir,
                    on_event=_handle_normalized_event,
                    on_turn_started=_handle_turn_started,
                    idle_timeout_seconds=CHAT_TURN_IDLE_TIMEOUT_SECONDS,
                    server_request_handler=_handle_server_request,
                )
            except RuntimeError:
                self._close_unlocked()
                raise
            for tool_call in tool_calls_by_id.values():
                if tool_call.status == "running":
                    tool_call.status = "failed" if result.state.last_error else "completed"
                    self._emit_live_event(
                        on_event,
                        TurnStreamEvent(
                            kind="tool_call_failed" if result.state.last_error else "tool_call_completed",
                            tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                            source=TurnStreamSource(
                                backend="codex_app_server",
                                app_turn_id=current_app_turn_id,
                                item_id=tool_call.id,
                                raw_kind="running_tool_call_reconciled",
                            ),
                        ),
                    )
            response_text = result.assistant_message or result.plan_message
            return ChatTurnResult(
                assistant_message=response_text or "",
                token_usage=copy.deepcopy(result.token_usage_payload) if result.token_usage_payload else None,
            )
