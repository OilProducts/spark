from __future__ import annotations

import json
import selectors
import subprocess
import threading
import time
import uuid
from pathlib import Path
from typing import Any, Callable, Optional

from sparkspawn_common import codex_app_server
from workspace.project_chat_common import (
    as_non_empty_string,
    build_codex_runtime_environment,
    normalize_project_path_value,
    resolve_runtime_workspace_path,
)
from workspace.project_chat_models import (
    ChatTurnLiveEvent,
    ChatTurnResult,
    ToolCallRecord,
)


CHAT_TURN_IDLE_TIMEOUT_SECONDS = codex_app_server.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS
FINAL_ANSWER_SETTLE_TIMEOUT_SECONDS = 0.5
APP_SERVER_REQUEST_TIMEOUT_SECONDS = 15.0
LEGACY_OPT_OUT_NOTIFICATION_METHODS = [
    "codex/event/agent_message",
    "codex/event/agent_message_delta",
    "codex/event/agent_message_content_delta",
    "codex/event/agent_reasoning_delta",
    "codex/event/reasoning_content_delta",
    "codex/event/item_started",
    "codex/event/item_completed",
    "codex/event/task_complete",
    "codex/event/token_count",
]


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


def _extract_app_turn_id(message: dict[str, Any]) -> Optional[str]:
    params = message.get("params")
    if not isinstance(params, dict):
        return None
    direct = as_non_empty_string(params.get("turnId"))
    if direct:
        return direct
    turn = params.get("turn")
    if isinstance(turn, dict):
        nested = as_non_empty_string(turn.get("id"))
        if nested:
            return nested
    nested_msg = params.get("msg")
    if isinstance(nested_msg, dict):
        nested = as_non_empty_string(nested_msg.get("turn_id") or nested_msg.get("turnId"))
        if nested:
            return nested
    params_id = as_non_empty_string(params.get("id"))
    if params_id and isinstance(message.get("method"), str) and str(message.get("method")).startswith("codex/event/"):
        return params_id
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
        self._proc: Optional[subprocess.Popen[str]] = None
        self._selector: Optional[selectors.DefaultSelector] = None
        self._request_id = 0
        self._thread_id: Optional[str] = persisted_thread_id
        self._thread_initialized = False
        self._on_thread_id_updated = on_thread_id_updated
        self._raw_rpc_logger: Optional[Callable[[str, str], None]] = None
        self._lock = threading.Lock()

    def _close(self) -> None:
        if self._selector is not None:
            try:
                self._selector.close()
            except Exception:
                pass
            self._selector = None
        if self._proc is not None:
            try:
                if self._proc.poll() is None:
                    self._proc.terminate()
                    try:
                        self._proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        self._proc.kill()
            except Exception:
                pass
            self._proc = None
        self._thread_initialized = False

    def close(self) -> None:
        with self._lock:
            self._close()

    def set_raw_rpc_logger(self, callback: Optional[Callable[[str, str], None]]) -> None:
        self._raw_rpc_logger = callback

    def clear_raw_rpc_logger(self) -> None:
        self._raw_rpc_logger = None

    def _ensure_process(self) -> None:
        if self._proc is not None and self._proc.poll() is None:
            return
        self._close()
        try:
            proc = subprocess.Popen(
                ["codex", "app-server"],
                cwd=self.working_dir,
                env=build_codex_runtime_environment(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                start_new_session=True,
            )
        except FileNotFoundError as exc:
            if not Path(self.working_dir).exists():
                raise RuntimeError(
                    "codex app-server working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir or self.working_dir}, resolved {self.working_dir}"
                ) from exc
            raise RuntimeError("codex app-server not found on PATH") from exc
        selector = selectors.DefaultSelector()
        if proc.stdout is None:
            self._close()
            raise RuntimeError("codex app-server did not expose stdout")
        selector.register(proc.stdout, selectors.EVENT_READ)
        self._proc = proc
        self._selector = selector
        self._request_id = 0
        self._thread_initialized = False
        init_response = self._send_request(
            "initialize",
            {
                "clientInfo": {"name": "sparkspawn", "version": "0.1"},
                "capabilities": {
                    "experimentalApi": True,
                    "optOutNotificationMethods": LEGACY_OPT_OUT_NOTIFICATION_METHODS,
                },
            },
        )
        if init_response.get("error"):
            self._close()
            raise RuntimeError("codex app-server initialize failed")
        self._send_json({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    def _send_json(self, payload: dict[str, Any]) -> None:
        if self._proc is None or self._proc.stdin is None:
            raise RuntimeError("codex app-server stdin unavailable")
        raw_line = json.dumps(payload)
        if self._raw_rpc_logger is not None:
            self._raw_rpc_logger("outgoing", raw_line)
        self._proc.stdin.write(raw_line + "\n")
        self._proc.stdin.flush()

    def _send_response(self, request_id: Any, result: Optional[dict[str, Any]] = None) -> None:
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result or {},
        })

    def _read_line(self, wait: float) -> Optional[str]:
        if self._proc is None or self._selector is None or self._proc.stdout is None:
            return None
        events = self._selector.select(timeout=max(wait, 0))
        if not events:
            return None
        line = self._proc.stdout.readline()
        if not line:
            return None
        return line.rstrip("\n")

    def _handle_server_request(self, message: dict[str, Any]) -> None:
        method = message.get("method")
        request_id = message.get("id")
        if method in {"item/commandExecution/requestApproval", "item/fileChange/requestApproval"}:
            self._send_response(request_id, {"decision": "acceptForSession"})
            return
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": f"Unsupported request: {method}"},
        })

    def _wait_for_response(self, target_id: int) -> dict[str, Any]:
        started_at = time.monotonic()
        while True:
            line = self._read_line(0.1)
            if line is None:
                if self._proc is not None and self._proc.poll() is not None:
                    raise RuntimeError("codex app-server exited unexpectedly")
                if time.monotonic() - started_at >= APP_SERVER_REQUEST_TIMEOUT_SECONDS:
                    self._close()
                    raise RuntimeError("codex app-server request timed out waiting for response")
                continue
            if self._raw_rpc_logger is not None:
                self._raw_rpc_logger("incoming", line)
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in message and "method" in message:
                self._handle_server_request(message)
                continue
            if message.get("id") == target_id:
                return message

    def _send_request(self, method: str, params: Optional[dict[str, Any]]) -> dict[str, Any]:
        self._request_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": self._request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self._send_json(payload)
        return self._wait_for_response(self._request_id)

    def _set_thread_id(self, thread_id: str) -> None:
        normalized_thread_id = as_non_empty_string(thread_id)
        if not normalized_thread_id:
            return
        self._thread_id = normalized_thread_id
        if self._on_thread_id_updated is not None:
            self._on_thread_id_updated(normalized_thread_id)

    def _resume_thread(self, model: Optional[str]) -> bool:
        if not self._thread_id:
            return False
        params: dict[str, Any] = {
            "threadId": self._thread_id,
            "cwd": self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
        }
        if model:
            params["model"] = model
        response = self._send_request("thread/resume", params)
        if response.get("error"):
            return False
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = thread.get("id")
        if not thread_id:
            return False
        self._set_thread_id(str(thread_id))
        self._thread_initialized = True
        return True

    def _start_thread(self, model: Optional[str]) -> None:
        params: dict[str, Any] = {
            "cwd": self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
            "ephemeral": False,
        }
        if model:
            params["model"] = model
        response = self._send_request("thread/start", params)
        if response.get("error"):
            message = as_non_empty_string((response.get("error") or {}).get("message"))
            if message:
                raise RuntimeError(f"codex app-server thread/start failed: {message}")
            raise RuntimeError("codex app-server thread/start failed")
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = thread.get("id")
        if not thread_id:
            raise RuntimeError("codex app-server did not return a thread id")
        self._set_thread_id(str(thread_id))
        self._thread_initialized = True

    def _ensure_thread(self, model: Optional[str]) -> None:
        if self._thread_initialized and self._thread_id:
            return
        if self._resume_thread(model):
            return
        self._start_thread(model)

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
            stream_state = codex_app_server.CodexAppServerTurnState()
            last_activity_at = time.monotonic()
            tool_calls_by_id: dict[str, ToolCallRecord] = {}
            current_app_turn_id: Optional[str] = None
            params: dict[str, Any] = {
                "threadId": self._thread_id,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": self.working_dir,
            }
            if model:
                params["model"] = model
            response = self._send_request("turn/start", params)
            if response.get("error"):
                raise RuntimeError("codex app-server turn/start failed")

            while True:
                line = self._read_line(0.1)
                if line is None:
                    idle_for = time.monotonic() - last_activity_at
                    if (
                        stream_state.can_finalize_without_turn_completed()
                        and idle_for >= FINAL_ANSWER_SETTLE_TIMEOUT_SECONDS
                    ):
                        break
                    if idle_for >= CHAT_TURN_IDLE_TIMEOUT_SECONDS:
                        if stream_state.can_finalize_without_turn_completed():
                            break
                        self._close()
                        raise RuntimeError("codex app-server turn timed out waiting for activity")
                    if self._proc is not None and self._proc.poll() is not None:
                        if stream_state.can_finalize_without_turn_completed():
                            break
                        self._close()
                        raise RuntimeError("codex app-server exited before turn completion")
                    continue
                last_activity_at = time.monotonic()
                if self._raw_rpc_logger is not None:
                    self._raw_rpc_logger("incoming", line)
                message = codex_app_server.parse_jsonrpc_line(line)
                if message is None:
                    continue
                extracted_turn_id = _extract_app_turn_id(message)
                if extracted_turn_id:
                    current_app_turn_id = extracted_turn_id
                if "id" in message and "method" in message:
                    request_method = message.get("method")
                    request_params = message.get("params") or {}
                    if request_method == "item/commandExecution/requestApproval":
                        command = codex_app_server.extract_command_text(request_params)
                        item_id = as_non_empty_string(request_params.get("itemId")) or f"tool-{uuid.uuid4().hex}"
                        tool_call = ToolCallRecord(
                            id=item_id,
                            kind="command_execution",
                            status="running",
                            title="Run command",
                            command=command,
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
                    elif request_method == "item/fileChange/requestApproval":
                        file_paths = codex_app_server.extract_file_paths(request_params)
                        item_id = as_non_empty_string(request_params.get("itemId")) or f"tool-{uuid.uuid4().hex}"
                        tool_call = ToolCallRecord(
                            id=item_id,
                            kind="file_change",
                            status="running",
                            title="Apply file changes",
                            file_paths=file_paths,
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
                    self._handle_server_request(message)
                    continue
                normalized_events = codex_app_server.process_turn_message(message, stream_state)
                for normalized_event in normalized_events:
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
                        continue
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
                        continue
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
                        continue
                    if normalized_event.kind == "tool_item_started" and isinstance(normalized_event.item, dict):
                        tool_call = _tool_call_from_item(normalized_event.item)
                        if tool_call is None:
                            continue
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
                        continue
                    if normalized_event.kind == "tool_item_completed" and isinstance(normalized_event.item, dict):
                        tool_call = _tool_call_from_item(normalized_event.item)
                        if tool_call is None:
                            continue
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
                        continue
                    if normalized_event.kind == "command_output_delta" and normalized_event.text:
                        tool_call = tool_calls_by_id.get(normalized_event.item_id or "")
                        if tool_call is None:
                            continue
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
                        continue
                    if normalized_event.kind == "turn_completed":
                        break
                if any(event.kind == "turn_completed" for event in normalized_events):
                    break
            for tool_call in tool_calls_by_id.values():
                if tool_call.status == "running":
                    tool_call.status = "failed" if stream_state.last_error else "completed"
                    self._emit_live_event(
                        on_event,
                        ChatTurnLiveEvent(
                            kind="tool_call_failed" if stream_state.last_error else "tool_call_completed",
                            tool_call_id=tool_call.id,
                            tool_call=ToolCallRecord.from_dict(tool_call.to_dict()),
                            app_turn_id=current_app_turn_id,
                        ),
                    )
            if stream_state.last_error:
                raise RuntimeError(stream_state.last_error)
            response_text = stream_state.resolved_agent_text()
            if not response_text:
                raise RuntimeError("codex app-server returned an empty chat response")
            return ChatTurnResult(assistant_message=response_text)
