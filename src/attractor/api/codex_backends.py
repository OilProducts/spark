from __future__ import annotations

import json
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Callable, Optional

from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.handlers.base import CodergenBackend
from spark_common.codex_app_client import CodexAppServerClient
from spark_common import codex_app_server
from spark_common.runtime import resolve_runtime_workspace_path

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
        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=reason)

        client = CodexAppServerClient(
            self.working_dir,
            requested_working_dir=self.requested_working_dir,
            on_unparsed_line=log_line,
        )
        try:
            client.ensure_process(popen_factory=subprocess.Popen)

            def start_thread() -> str | None:
                try:
                    return client.start_thread(
                        model=self.model,
                        cwd=self.working_dir,
                        ephemeral=True,
                    )
                except RuntimeError:
                    return None

            thread_key = self._runtime_thread_key(context)
            thread_uuid = self._resolve_session_thread_id(thread_key, start_thread)
            if not thread_uuid:
                return fail("codex app-server thread/start failed")

            result = client.run_turn(
                thread_id=thread_uuid,
                prompt=prompt,
                model=self.model,
                cwd=self.working_dir,
                overall_timeout_seconds=timeout,
                now=time.monotonic,
            )
            agent_text = result.assistant_message
            if agent_text:
                log_line(agent_text)
            command_text = result.command_text
            if command_text:
                log_line(command_text)
            if result.token_total is not None:
                log_line(f"tokens used: {result.token_total}")
            if agent_text:
                return _coerce_structured_text_outcome(agent_text)
            if command_text:
                return _coerce_structured_text_outcome(command_text)
            return "codex app-server completed successfully"
        except RuntimeError as exc:
            return fail(str(exc))
        finally:
            client.close()


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
