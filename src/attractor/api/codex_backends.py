from __future__ import annotations

import json
from collections import deque
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Callable, Optional

from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.handlers.base import CodergenBackend
from spark_common import codex_app_server
from spark_common.process_line_reader import ProcessLineReader
from spark_common.runtime import build_codex_runtime_environment, resolve_runtime_workspace_path

_STRUCTURED_OUTCOME_KEYS = {
    "outcome",
    "preferred_label",
    "preferred_next_label",
    "suggested_next_ids",
    "context_updates",
    "notes",
    "failure_reason",
    "retryable",
}


def _extract_turn_id(message: dict[str, Any]) -> str | None:
    params = message.get("params")
    if not isinstance(params, dict):
        return None
    direct = codex_app_server.as_non_empty_string(params.get("turnId"))
    if direct:
        return direct
    turn = params.get("turn")
    if isinstance(turn, dict):
        nested = codex_app_server.as_non_empty_string(turn.get("id"))
        if nested:
            return nested
    return None


class CodexAppServerBackend(CodergenBackend):
    RUNTIME_THREAD_ID_KEY = "_attractor.runtime.thread_id"

    def __init__(self, working_dir: str, emit, model: Optional[str] = None):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.model = model
        self._session_threads_by_key: dict[str, str] = {}
        self._session_threads_lock = threading.Lock()

    def _runtime_thread_key(self, context: Context) -> str:
        value = context.get(self.RUNTIME_THREAD_ID_KEY, "")
        if value is None:
            return ""
        return str(value).strip()

    def _resolve_session_thread_id(
        self,
        thread_key: str,
        start_thread: Callable[[], str | None],
    ) -> str | None:
        normalized_key = thread_key.strip()
        if not normalized_key:
            return start_thread()

        with self._session_threads_lock:
            cached = self._session_threads_by_key.get(normalized_key)
            if cached:
                return cached
            created = start_thread()
            if not created:
                return None
            self._session_threads_by_key[normalized_key] = created
            return created

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> str | Outcome:
        cmd = ["codex", "app-server"]
        deadline = time.monotonic() + timeout if timeout else None
        last_activity_at = time.monotonic()
        stream_state = codex_app_server.CodexAppServerTurnState()
        pending_messages: deque[dict[str, Any]] = deque()

        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=reason)

        try:
            proc = subprocess.Popen(
                cmd,
                cwd=self.working_dir,
                env=build_codex_runtime_environment(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                start_new_session=True,
            )
        except FileNotFoundError:
            if not Path(self.working_dir).exists():
                return fail(
                    "codex app-server working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir}, resolved {self.working_dir}"
                )
            return fail("codex app-server not found on PATH")

        stdout_reader = ProcessLineReader(proc.stdout) if proc.stdout is not None else None

        def send_json(payload: dict) -> None:
            if proc.stdin is None:
                return
            proc.stdin.write(json.dumps(payload) + "\n")
            proc.stdin.flush()

        request_id = 0

        def send_request(method: str, params: Optional[dict]) -> int:
            nonlocal request_id
            request_id += 1
            payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
            if params is not None:
                payload["params"] = params
            send_json(payload)
            return request_id

        def send_response(req_id: object, result: Optional[dict] = None, error: Optional[dict] = None) -> None:
            payload = {"jsonrpc": "2.0", "id": req_id}
            if error is not None:
                payload["error"] = error
            else:
                payload["result"] = result or {}
            send_json(payload)

        def read_line(wait: float) -> Optional[str]:
            if stdout_reader is None:
                return None
            return stdout_reader.read_line(wait)

        def handle_server_request(message: dict) -> None:
            method = message.get("method")
            req_id = message.get("id")
            if method == "item/commandExecution/requestApproval":
                send_response(req_id, {"decision": "acceptForSession"})
                return
            if method == "item/fileChange/requestApproval":
                send_response(req_id, {"decision": "acceptForSession"})
                return
            send_response(req_id, error={"code": -32000, "message": f"Unsupported request: {method}"})

        def wait_for_response(target_id: int) -> Optional[dict]:
            while True:
                if deadline is not None and time.monotonic() > deadline:
                    return None
                line = read_line(0.1)
                if line is None:
                    if proc.poll() is not None:
                        return None
                    continue
                message = codex_app_server.parse_jsonrpc_line(line)
                if message is None:
                    log_line(line)
                    continue
                if "id" in message and "method" in message:
                    pending_messages.append(
                        {
                            "jsonrpc": message.get("jsonrpc", "2.0"),
                            "method": message.get("method"),
                            "params": message.get("params") or {},
                        }
                    )
                    handle_server_request(message)
                    continue
                if message.get("id") == target_id:
                    return message
                if "method" in message:
                    pending_messages.append(message)

        def next_message(wait: float) -> Optional[dict]:
            if pending_messages:
                return pending_messages.popleft()
            deadline_at = time.monotonic() + max(wait, 0)
            while True:
                remaining = deadline_at - time.monotonic()
                if remaining <= 0:
                    return None
                line = read_line(remaining)
                if line is None:
                    return None
                message = codex_app_server.parse_jsonrpc_line(line)
                if message is None:
                    log_line(line)
                    continue
                if "id" in message and "method" in message:
                    handle_server_request(message)
                    return {
                        "jsonrpc": message.get("jsonrpc", "2.0"),
                        "method": message.get("method"),
                        "params": message.get("params") or {},
                    }
                if "method" in message:
                    return message

        try:
            init_id = send_request(
                "initialize",
                {"clientInfo": {"name": "spark", "version": "0.1"}},
            )
            init_response = wait_for_response(init_id)
            if not init_response or init_response.get("error"):
                return fail("app-server initialize failed")
            # Per the app-server protocol, the client must acknowledge the `initialize`
            # handshake with an `initialized` notification before other requests.
            send_json({"jsonrpc": "2.0", "method": "initialized", "params": {}})

            def start_thread() -> str | None:
                thread_params = {
                    "cwd": self.working_dir,
                    "sandbox": "danger-full-access",
                    "ephemeral": True,
                }
                if self.model:
                    thread_params["model"] = self.model
                thread_request_id = send_request("thread/start", thread_params)
                thread_response = wait_for_response(thread_request_id)
                if not thread_response or thread_response.get("error"):
                    return None
                thread = (thread_response.get("result") or {}).get("thread") or {}
                thread_uuid = thread.get("id")
                if not thread_uuid:
                    return None
                return str(thread_uuid)

            thread_key = self._runtime_thread_key(context)
            thread_uuid = self._resolve_session_thread_id(thread_key, start_thread)
            if not thread_uuid:
                return fail("app-server thread/start failed")

            turn_params = {
                "threadId": thread_uuid,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": self.working_dir,
            }
            if self.model:
                turn_params["model"] = self.model
            turn_request_id = send_request("turn/start", turn_params)
            turn_response = wait_for_response(turn_request_id)
            if not turn_response or turn_response.get("error"):
                return fail("app-server turn/start failed")
            turn = (turn_response.get("result") or {}).get("turn") or {}
            expected_turn_id = codex_app_server.as_non_empty_string(turn.get("id"))
            if not expected_turn_id:
                return fail("app-server turn/start did not return a turn id")

            while True:
                if deadline is not None and time.monotonic() > deadline:
                    return fail(f"app-server turn timed out after {timeout:g}s")
                message = next_message(0.1)
                if message is None:
                    idle_for = time.monotonic() - last_activity_at
                    if idle_for >= codex_app_server.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS:
                        return fail("app-server turn timed out waiting for activity")
                    if proc.poll() is not None:
                        return fail("app-server turn exited before completion")
                    continue
                last_activity_at = time.monotonic()
                extracted_turn_id = _extract_turn_id(message)
                if extracted_turn_id and extracted_turn_id != expected_turn_id:
                    continue
                if message.get("method") in {"item/commandExecution/requestApproval", "item/fileChange/requestApproval"}:
                    continue
                normalized_events = codex_app_server.process_turn_message(message, stream_state)
                if any(event.kind == "turn_completed" for event in normalized_events) and extracted_turn_id == expected_turn_id:
                    break

            if stream_state.turn_status and stream_state.turn_status != "completed":
                return fail(stream_state.turn_error or f"app-server turn ended with status '{stream_state.turn_status}'")
            if stream_state.last_error:
                return fail(stream_state.last_error)

            agent_text = stream_state.resolved_agent_text()
            if agent_text:
                log_line(agent_text)
            command_text = stream_state.resolved_command_text()
            if command_text:
                log_line(command_text)
            if stream_state.last_token_total is not None:
                log_line(f"tokens used: {stream_state.last_token_total}")
            if agent_text:
                return _coerce_structured_text_outcome(agent_text)
            if command_text:
                return _coerce_structured_text_outcome(command_text)
            return "codex app-server completed successfully"
        finally:
            try:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        proc.kill()
            except Exception:
                pass
            if stdout_reader is not None:
                try:
                    stdout_reader.join(timeout=0.5)
                except Exception:
                    pass


def build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
) -> CodergenBackend:
    normalized = backend_name.strip().lower()
    if normalized == "codex-app-server":
        return CodexAppServerBackend(working_dir, emit, model=model)
    raise ValueError(
        "Unsupported backend. Supported backends: codex-app-server."
    )


def _coerce_structured_text_outcome(text: str) -> str | Outcome:
    candidate = _extract_structured_outcome_payload(text)
    if candidate is None:
        return text

    preferred_label = candidate.get("preferred_label")
    if preferred_label is None:
        preferred_label = candidate.get("preferred_next_label", "")
    suggested_next_ids = candidate.get("suggested_next_ids", [])
    context_updates = candidate.get("context_updates", {})
    notes = candidate.get("notes", "")
    failure_reason = candidate.get("failure_reason", "")
    retryable = candidate.get("retryable", None)

    if not isinstance(preferred_label, str):
        return text
    if not isinstance(suggested_next_ids, list) or any(not isinstance(item, str) for item in suggested_next_ids):
        return text
    if not isinstance(context_updates, dict):
        return text
    if notes is not None and not isinstance(notes, str):
        return text
    if failure_reason is not None and not isinstance(failure_reason, str):
        return text
    if retryable is not None and not isinstance(retryable, bool):
        return text

    outcome_name = str(candidate.get("outcome", "")).strip().lower()
    try:
        status = OutcomeStatus(outcome_name)
    except ValueError:
        return text

    if status == OutcomeStatus.SKIPPED:
        return text

    return Outcome(
        status=status,
        preferred_label=preferred_label,
        suggested_next_ids=list(suggested_next_ids),
        context_updates=dict(context_updates),
        notes=notes or "",
        failure_reason=failure_reason or "",
        retryable=retryable,
    )


def _extract_structured_outcome_payload(text: str) -> dict[str, object] | None:
    stripped = text.strip()
    if not stripped:
        return None

    candidates = [stripped]
    if stripped.startswith("```") and stripped.endswith("```"):
        lines = stripped.splitlines()
        if len(lines) >= 3:
            candidates.append("\n".join(lines[1:-1]).strip())

    for candidate in candidates:
        try:
            payload = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if not isinstance(payload, dict):
            continue
        if "outcome" not in payload:
            continue
        if not set(payload.keys()).issubset(_STRUCTURED_OUTCOME_KEYS):
            continue
        return payload
    return None
