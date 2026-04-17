from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import json
from pathlib import Path
import subprocess
import time
from typing import Any, Callable, Optional

from spark_common import codex_app_server
from spark_common.codex_runtime import build_codex_runtime_environment
from spark_common.process_line_reader import ProcessLineReader


APP_SERVER_REQUEST_TIMEOUT_SECONDS = 15.0
COLLABORATION_MODE_DEFAULT = "default"
COLLABORATION_MODE_PLAN = "plan"


def _collaboration_mode_for_chat_mode(chat_mode: Optional[str], model: str) -> dict[str, Any]:
    normalized = str(chat_mode or "").strip().lower()
    collaboration_mode = COLLABORATION_MODE_PLAN if normalized == "plan" else COLLABORATION_MODE_DEFAULT
    return {
        "mode": collaboration_mode,
        "settings": {
            "model": model,
        },
    }


@dataclass
class CodexAppServerTurnResult:
    thread_id: str
    turn_id: str
    state: codex_app_server.CodexAppServerTurnState

    @property
    def assistant_message(self) -> str:
        return self.state.resolved_agent_text()

    @property
    def plan_message(self) -> str:
        return self.state.resolved_plan_text()

    @property
    def command_text(self) -> str:
        return self.state.resolved_command_text()

    @property
    def token_total(self) -> Optional[int]:
        return self.state.last_token_total

    @property
    def token_usage_payload(self) -> Optional[dict[str, Any]]:
        return self.state.last_token_usage_payload


class CodexAppServerClient:
    def __init__(
        self,
        working_dir: str,
        *,
        requested_working_dir: Optional[str] = None,
        request_timeout_seconds: Optional[float] = APP_SERVER_REQUEST_TIMEOUT_SECONDS,
        on_unparsed_line: Optional[Callable[[str], None]] = None,
    ) -> None:
        self.requested_working_dir = requested_working_dir or working_dir
        self.working_dir = working_dir
        self.request_timeout_seconds = request_timeout_seconds
        self._on_unparsed_line = on_unparsed_line
        self._proc: Optional[subprocess.Popen[str]] = None
        self._stdout_reader: Optional[ProcessLineReader] = None
        self._request_id = 0
        self._pending_messages: deque[dict[str, Any]] = deque()
        self._raw_rpc_logger: Optional[Callable[[str, str], None]] = None

    @property
    def proc(self) -> Optional[subprocess.Popen[str]]:
        return self._proc

    @proc.setter
    def proc(self, value: Optional[subprocess.Popen[str]]) -> None:
        self._proc = value

    @property
    def pending_messages(self) -> deque[dict[str, Any]]:
        return self._pending_messages

    def close(self) -> None:
        stdout_reader = self._stdout_reader
        self._stdout_reader = None
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
        if stdout_reader is not None:
            try:
                stdout_reader.join(timeout=0.5)
            except Exception:
                pass
        self._pending_messages.clear()
        self._request_id = 0

    def set_raw_rpc_logger(self, callback: Optional[Callable[[str, str], None]]) -> None:
        self._raw_rpc_logger = callback

    def clear_raw_rpc_logger(self) -> None:
        self._raw_rpc_logger = None

    def ensure_process(
        self,
        *,
        popen_factory: Callable[..., subprocess.Popen[str]] = subprocess.Popen,
        runtime_environment_builder: Callable[[], dict[str, str]] = build_codex_runtime_environment,
    ) -> None:
        if self._proc is not None and self._proc.poll() is None:
            return
        self.close()
        try:
            proc = popen_factory(
                ["codex", "app-server"],
                cwd=self.working_dir,
                env=runtime_environment_builder(),
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
                    f"requested {self.requested_working_dir}, resolved {self.working_dir}"
                ) from exc
            raise RuntimeError("codex app-server not found on PATH") from exc
        if proc.stdout is None:
            self.close()
            raise RuntimeError("codex app-server did not expose stdout")
        self._proc = proc
        self._stdout_reader = ProcessLineReader(proc.stdout)
        self._request_id = 0
        self._pending_messages.clear()
        init_response = self.send_request(
            "initialize",
            {
                "clientInfo": {"name": "spark", "version": "0.1"},
                "capabilities": {"experimentalApi": True},
            },
        )
        if init_response.get("error"):
            self.close()
            raise RuntimeError("codex app-server initialize failed")
        self.send_json({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    def send_json(self, payload: dict[str, Any]) -> None:
        if self._proc is None or self._proc.stdin is None:
            raise RuntimeError("codex app-server stdin unavailable")
        raw_line = json.dumps(payload)
        if self._raw_rpc_logger is not None:
            self._raw_rpc_logger("outgoing", raw_line)
        self._proc.stdin.write(raw_line + "\n")
        self._proc.stdin.flush()

    def send_response(
        self,
        request_id: Any,
        result: Optional[dict[str, Any]] = None,
        error: Optional[dict[str, Any]] = None,
    ) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id}
        if error is not None:
            payload["error"] = error
        else:
            payload["result"] = result or {}
        self.send_json(payload)

    def read_line(self, wait: float) -> Optional[str]:
        if self._proc is None or self._stdout_reader is None:
            return None
        return self._stdout_reader.read_line(wait)

    def _log_incoming_line(self, line: str) -> None:
        if self._raw_rpc_logger is not None:
            self._raw_rpc_logger("incoming", line)

    def _handle_unparsed_line(self, line: str) -> None:
        if self._on_unparsed_line is not None:
            self._on_unparsed_line(line)

    def _handle_server_request(self, message: dict[str, Any]) -> dict[str, Any]:
        method = message.get("method")
        request_id = message.get("id")
        if method in {"item/commandExecution/requestApproval", "item/fileChange/requestApproval"}:
            self.send_response(request_id, {"decision": "acceptForSession"})
        else:
            self.send_response(
                request_id,
                error={"code": -32000, "message": f"Unsupported request: {method}"},
            )
        return {
            "jsonrpc": message.get("jsonrpc", "2.0"),
            "method": method,
            "params": message.get("params") or {},
        }

    def wait_for_response(
        self,
        target_id: int,
        *,
        read_line: Optional[Callable[[float], Optional[str]]] = None,
        now: Callable[[], float] = time.monotonic,
    ) -> dict[str, Any]:
        started_at = now()
        reader = read_line or self.read_line
        while True:
            line = reader(0.1)
            if line is None:
                if self._proc is not None and self._proc.poll() is not None:
                    raise RuntimeError("codex app-server exited unexpectedly")
                if self.request_timeout_seconds is not None and now() - started_at >= self.request_timeout_seconds:
                    self.close()
                    raise RuntimeError("codex app-server request timed out waiting for response")
                continue
            self._log_incoming_line(line)
            message = codex_app_server.parse_jsonrpc_line(line)
            if message is None:
                self._handle_unparsed_line(line)
                continue
            if "id" in message and "method" in message:
                self._pending_messages.append(self._handle_server_request(message))
                continue
            if message.get("id") == target_id:
                return message
            if "method" in message:
                self._pending_messages.append(message)

    def next_message(
        self,
        wait: float,
        *,
        read_line: Optional[Callable[[float], Optional[str]]] = None,
        now: Callable[[], float] = time.monotonic,
        on_activity: Optional[Callable[[], None]] = None,
        server_request_handler: Optional[Callable[[dict[str, Any]], dict[str, Any]]] = None,
    ) -> Optional[dict[str, Any]]:
        if self._pending_messages:
            return self._pending_messages.popleft()
        reader = read_line or self.read_line
        deadline = now() + max(wait, 0)
        while True:
            remaining = deadline - now()
            if remaining <= 0:
                return None
            line = reader(remaining)
            if line is None:
                return None
            if on_activity is not None:
                on_activity()
            self._log_incoming_line(line)
            message = codex_app_server.parse_jsonrpc_line(line)
            if message is None:
                self._handle_unparsed_line(line)
                continue
            if "id" in message and "method" in message:
                handler = server_request_handler or self._handle_server_request
                return handler(message)
            if "method" in message:
                return message

    def send_request(
        self,
        method: str,
        params: Optional[dict[str, Any]],
        *,
        read_line: Optional[Callable[[float], Optional[str]]] = None,
        now: Callable[[], float] = time.monotonic,
    ) -> dict[str, Any]:
        self._request_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": self._request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self.send_json(payload)
        return self.wait_for_response(self._request_id, read_line=read_line, now=now)

    def start_thread(
        self,
        *,
        model: Optional[str],
        cwd: Optional[str] = None,
        approval_policy: str = "never",
        ephemeral: bool,
        read_line: Optional[Callable[[float], Optional[str]]] = None,
    ) -> str:
        params: dict[str, Any] = {
            "cwd": cwd or self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": approval_policy,
            "ephemeral": ephemeral,
        }
        if model:
            params["model"] = model
        response = self.send_request("thread/start", params, read_line=read_line)
        if response.get("error"):
            message = codex_app_server.as_non_empty_string((response.get("error") or {}).get("message"))
            if message:
                raise RuntimeError(f"codex app-server thread/start failed: {message}")
            raise RuntimeError("codex app-server thread/start failed")
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = codex_app_server.as_non_empty_string(thread.get("id"))
        if not thread_id:
            raise RuntimeError("codex app-server did not return a thread id")
        return thread_id

    def resume_thread(
        self,
        thread_id: str,
        *,
        model: Optional[str],
        cwd: Optional[str] = None,
        approval_policy: str = "never",
        read_line: Optional[Callable[[float], Optional[str]]] = None,
    ) -> Optional[str]:
        params: dict[str, Any] = {
            "threadId": thread_id,
            "cwd": cwd or self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": approval_policy,
        }
        if model:
            params["model"] = model
        response = self.send_request("thread/resume", params, read_line=read_line)
        if response.get("error"):
            return None
        thread = (response.get("result") or {}).get("thread") or {}
        return codex_app_server.as_non_empty_string(thread.get("id"))

    def default_model(self) -> Optional[str]:
        response = self.send_request("model/list", {"limit": 100})
        if response.get("error"):
            return None
        payload = response.get("result") or {}
        data = payload.get("data")
        if not isinstance(data, list):
            return None
        for entry in data:
            if not isinstance(entry, dict):
                continue
            if not entry.get("isDefault"):
                continue
            model = codex_app_server.as_non_empty_string(entry.get("model"))
            if model:
                return model
        return None

    def run_turn(
        self,
        *,
        thread_id: str,
        prompt: str,
        model: Optional[str],
        chat_mode: Optional[str] = None,
        cwd: Optional[str] = None,
        on_event: Optional[Callable[[codex_app_server.CodexAppServerTurnEvent], None]] = None,
        on_turn_started: Optional[Callable[[str], None]] = None,
        idle_timeout_seconds: float = codex_app_server.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS,
        overall_timeout_seconds: Optional[float] = None,
        send_request: Optional[Callable[[str, Optional[dict[str, Any]]], dict[str, Any]]] = None,
        next_message: Optional[Callable[[float], Optional[dict[str, Any]]]] = None,
        now: Callable[[], float] = time.monotonic,
        server_request_handler: Optional[Callable[[dict[str, Any]], dict[str, Any]]] = None,
    ) -> CodexAppServerTurnResult:
        request = send_request or self.send_request
        params: dict[str, Any] = {
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt}],
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"},
            "cwd": cwd or self.working_dir,
        }
        effective_model = codex_app_server.as_non_empty_string(model)
        if chat_mode is not None:
            effective_model = effective_model or self.default_model()
            if not effective_model:
                raise RuntimeError("codex app-server turn/start requires a resolved model")
            params["collaborationMode"] = _collaboration_mode_for_chat_mode(chat_mode, effective_model)
            params["model"] = effective_model
        elif effective_model:
            params["model"] = effective_model
        response = request("turn/start", params)
        if response.get("error"):
            message = codex_app_server.as_non_empty_string((response.get("error") or {}).get("message"))
            if message:
                raise RuntimeError(f"codex app-server turn/start failed: {message}")
            raise RuntimeError("codex app-server turn/start failed")
        turn = (response.get("result") or {}).get("turn") or {}
        expected_turn_id = codex_app_server.as_non_empty_string(turn.get("id"))
        if not expected_turn_id:
            raise RuntimeError("codex app-server did not return a turn id")
        return self._consume_turn_stream(
            thread_id=thread_id,
            expected_turn_id=expected_turn_id,
            on_event=on_event,
            on_turn_started=on_turn_started,
            idle_timeout_seconds=idle_timeout_seconds,
            overall_timeout_seconds=overall_timeout_seconds,
            next_message=next_message,
            now=now,
            server_request_handler=server_request_handler,
        )

    def _consume_turn_stream(
        self,
        *,
        thread_id: str,
        expected_turn_id: Optional[str],
        on_event: Optional[Callable[[codex_app_server.CodexAppServerTurnEvent], None]] = None,
        on_turn_started: Optional[Callable[[str], None]] = None,
        idle_timeout_seconds: float = codex_app_server.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS,
        overall_timeout_seconds: Optional[float] = None,
        next_message: Optional[Callable[[float], Optional[dict[str, Any]]]] = None,
        now: Callable[[], float] = time.monotonic,
        server_request_handler: Optional[Callable[[dict[str, Any]], dict[str, Any]]] = None,
    ) -> CodexAppServerTurnResult:
        stream_state = codex_app_server.CodexAppServerTurnState()
        started_at = now()
        last_activity_at = started_at

        def mark_activity() -> None:
            nonlocal last_activity_at
            last_activity_at = now()

        if next_message is None:
            def read_message(wait: float) -> Optional[dict[str, Any]]:
                return self.next_message(
                    wait,
                    now=now,
                    on_activity=mark_activity,
                    server_request_handler=server_request_handler,
                )
        else:
            read_message = next_message
        if expected_turn_id and on_turn_started is not None:
            on_turn_started(expected_turn_id)
        while True:
            if overall_timeout_seconds is not None and now() - started_at > overall_timeout_seconds:
                raise RuntimeError(f"codex app-server turn timed out after {overall_timeout_seconds:g}s")
            message = read_message(0.1)
            if message is None:
                idle_for = now() - last_activity_at
                if idle_for >= idle_timeout_seconds:
                    raise RuntimeError("codex app-server turn timed out waiting for activity")
                if self._proc is not None and self._proc.poll() is not None:
                    raise RuntimeError("codex app-server exited before turn completion")
                continue
            mark_activity()
            extracted_turn_id = codex_app_server.extract_turn_id(message)
            if expected_turn_id is None and extracted_turn_id:
                expected_turn_id = extracted_turn_id
                if on_turn_started is not None:
                    on_turn_started(expected_turn_id)
            if extracted_turn_id and extracted_turn_id != expected_turn_id:
                continue
            normalized_events = codex_app_server.process_turn_message(message, stream_state)
            for event in normalized_events:
                if on_event is not None:
                    on_event(event)
            if any(event.kind == "turn_completed" for event in normalized_events) and extracted_turn_id == expected_turn_id:
                break
        if not expected_turn_id:
            raise RuntimeError("codex app-server did not expose a resumable turn id")
        if stream_state.turn_status and stream_state.turn_status != "completed":
            raise RuntimeError(
                stream_state.turn_error or f"codex app-server turn ended with status '{stream_state.turn_status}'"
            )
        if stream_state.last_error:
            raise RuntimeError(stream_state.last_error)
        return CodexAppServerTurnResult(
            thread_id=thread_id,
            turn_id=expected_turn_id,
            state=stream_state,
        )
