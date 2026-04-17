from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import subprocess
import threading
import tomllib
import uuid
from typing import Any, Callable, Optional

from spark.workspace.conversations.models import (
    ChatTurnLiveEvent,
    ChatTurnResult,
    RequestUserInputOption,
    RequestUserInputQuestion,
    RequestUserInputRecord,
    ToolCallRecord,
)
from spark.workspace.conversations.utils import (
    as_non_empty_string,
    normalize_project_path_value,
)
from spark_common.codex_app_client import (
    APP_SERVER_REQUEST_TIMEOUT_SECONDS,
    CodexAppServerClient,
)
from spark_common import codex_app_server
from spark_common.codex_runtime import build_codex_runtime_environment
from spark_common.runtime_path import resolve_runtime_workspace_path


CHAT_TURN_IDLE_TIMEOUT_SECONDS = codex_app_server.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS


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
        command = codex_app_server.extract_command_text(item)
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
            file_paths=codex_app_server.extract_file_paths(item),
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
        callback: Optional[Callable[[ChatTurnLiveEvent], None]],
        event: ChatTurnLiveEvent,
    ) -> None:
        if callback is None:
            return
        callback(event)

    def _handle_request_user_input_server_request(
        self,
        message: dict[str, Any],
        *,
        on_event: Optional[Callable[[ChatTurnLiveEvent], None]],
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
            ChatTurnLiveEvent(
                kind="request_user_input_requested",
                app_turn_id=current_app_turn_id,
                item_id=request.request_id,
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
        normalized_event: codex_app_server.CodexAppServerTurnEvent,
        *,
        on_event: Optional[Callable[[ChatTurnLiveEvent], None]],
        tool_calls_by_id: dict[str, ToolCallRecord],
        current_app_turn_id: Optional[str],
    ) -> None:
        if normalized_event.kind == "command_approval_requested":
            payload = normalized_event.item or {}
            item_id = normalized_event.item_id or f"tool-{uuid.uuid4().hex}"
            tool_call = ToolCallRecord(
                id=item_id,
                kind="command_execution",
                status="running",
                title="Run command",
                command=normalized_event.text or codex_app_server.extract_command_text(payload),
            )
            tool_calls_by_id[item_id] = tool_call
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="tool_call_started",
                    tool_call_id=item_id,
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    app_turn_id=current_app_turn_id,
                    item_id=item_id,
                ),
            )
            return
        if normalized_event.kind == "file_change_approval_requested":
            payload = normalized_event.item or {}
            item_id = normalized_event.item_id or f"tool-{uuid.uuid4().hex}"
            tool_call = ToolCallRecord(
                id=item_id,
                kind="file_change",
                status="running",
                title="Apply file changes",
                file_paths=codex_app_server.extract_file_paths(payload),
            )
            tool_calls_by_id[item_id] = tool_call
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="tool_call_started",
                    tool_call_id=item_id,
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    app_turn_id=current_app_turn_id,
                    item_id=item_id,
                ),
            )
            return
        if normalized_event.kind == "assistant_delta" and normalized_event.text:
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="assistant_delta",
                    content_delta=normalized_event.text,
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                    phase=normalized_event.phase,
                ),
            )
            return
        if normalized_event.kind == "reasoning_delta" and normalized_event.text:
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="reasoning_summary",
                    content_delta=normalized_event.text,
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                    summary_index=normalized_event.summary_index,
                ),
            )
            return
        if normalized_event.kind == "plan_delta" and normalized_event.text:
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="plan_delta",
                    content_delta=normalized_event.text,
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "plan_completed" and normalized_event.text:
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="plan_completed",
                    content_delta=normalized_event.text,
                    message="Plan item completed.",
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "context_compaction_started":
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="context_compaction_started",
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "context_compaction_completed":
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="context_compaction_completed",
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "request_user_input_requested" and isinstance(normalized_event.item, dict):
            request = _request_user_input_record_from_payload(normalized_event.item)
            if request is None:
                return
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="request_user_input_requested",
                    app_turn_id=current_app_turn_id,
                    item_id=request.request_id,
                    request_user_input=request,
                ),
            )
            return
        if normalized_event.kind == "assistant_message_completed" and normalized_event.text:
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="assistant_completed",
                    content_delta=normalized_event.text,
                    message="Assistant message completed.",
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                    phase=normalized_event.phase,
                ),
            )
            return
        if normalized_event.kind == "tool_item_started" and isinstance(normalized_event.item, dict):
            tool_call = _tool_call_from_item(normalized_event.item)
            if tool_call is None:
                return
            if normalized_event.item_id:
                tool_calls_by_id[normalized_event.item_id] = tool_call
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="tool_call_started",
                    tool_call_id=normalized_event.item_id or tool_call.id,
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "tool_item_completed" and isinstance(normalized_event.item, dict):
            tool_call = _tool_call_from_item(normalized_event.item)
            if tool_call is None:
                return
            if normalized_event.item_id:
                tool_calls_by_id[normalized_event.item_id] = tool_call
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="tool_call_failed" if tool_call.status == "failed" else "tool_call_completed",
                    tool_call_id=normalized_event.item_id or tool_call.id,
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id,
                ),
            )
            return
        if normalized_event.kind == "command_output_delta" and normalized_event.text:
            tool_call = tool_calls_by_id.get(normalized_event.item_id or "")
            if tool_call is None:
                return
            tool_call.output = codex_app_server.append_tool_output(tool_call.output, normalized_event.text)
            self._emit_live_event(
                on_event,
                ChatTurnLiveEvent(
                    kind="tool_call_updated",
                    tool_call_id=tool_call.id,
                    tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                    app_turn_id=current_app_turn_id,
                    item_id=normalized_event.item_id or tool_call.id,
                ),
            )

    def turn(
        self,
        prompt: str,
        model: Optional[str],
        *,
        chat_mode: str = "chat",
        on_event: Optional[Callable[[ChatTurnLiveEvent], None]] = None,
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

            def _handle_normalized_event(normalized_event: codex_app_server.CodexAppServerTurnEvent) -> None:
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
                        ChatTurnLiveEvent(
                            kind="tool_call_failed" if result.state.last_error else "tool_call_completed",
                            tool_call_id=tool_call.id,
                            tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                            app_turn_id=current_app_turn_id,
                        ),
                    )
            response_text = result.assistant_message or result.plan_message
            return ChatTurnResult(assistant_message=response_text or "")
