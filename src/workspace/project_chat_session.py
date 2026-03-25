from __future__ import annotations

import subprocess
import threading
import uuid
from typing import Any, Callable, Optional

from spark_common.codex_app_client import (
    APP_SERVER_REQUEST_TIMEOUT_SECONDS,
    CodexAppServerClient,
)
from spark_common import codex_app_server
from workspace.project_chat_common import (
    as_non_empty_string,
    normalize_project_path_value,
    resolve_runtime_workspace_path,
)
from workspace.project_chat_models import (
    ChatTurnLiveEvent,
    ChatTurnResult,
    ToolCallRecord,
)


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


class CodexAppServerChatSession:
    def __init__(
        self,
        working_dir: str,
        *,
        persisted_thread_id: Optional[str] = None,
        on_thread_id_updated: Optional[Callable[[str], None]] = None,
    ) -> None:
        self.requested_working_dir = normalize_project_path_value(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self._thread_id: Optional[str] = persisted_thread_id
        self._thread_initialized = False
        self._on_thread_id_updated = on_thread_id_updated
        self._client = CodexAppServerClient(
            self.working_dir,
            requested_working_dir=self.requested_working_dir or self.working_dir,
            request_timeout_seconds=APP_SERVER_REQUEST_TIMEOUT_SECONDS,
        )
        self._lock = threading.Lock()

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

    def turn(
        self,
        prompt: str,
        model: Optional[str],
        *,
        on_event: Optional[Callable[[ChatTurnLiveEvent], None]] = None,
    ) -> ChatTurnResult:
        with self._lock:
            self._ensure_process()
            self._ensure_thread(model)
            tool_calls_by_id: dict[str, ToolCallRecord] = {}
            current_app_turn_id: Optional[str] = None
            def _handle_turn_started(turn_id: str) -> None:
                nonlocal current_app_turn_id
                current_app_turn_id = turn_id

            def _handle_normalized_event(normalized_event: codex_app_server.CodexAppServerTurnEvent) -> None:
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

            try:
                result = self._client.run_turn(
                    thread_id=self._thread_id or "",
                    prompt=prompt,
                    model=model,
                    cwd=self.working_dir,
                    on_event=_handle_normalized_event,
                    on_turn_started=_handle_turn_started,
                    idle_timeout_seconds=CHAT_TURN_IDLE_TIMEOUT_SECONDS,
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
            response_text = result.assistant_message
            if not response_text:
                raise RuntimeError("codex app-server returned an empty chat response")
            return ChatTurnResult(assistant_message=response_text)
