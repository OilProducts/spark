from __future__ import annotations

import copy
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import threading
import tomllib
import uuid
from typing import Any, Callable, Mapping, Optional, Protocol
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
    CodexAppServerThreadResumeFailure,
)
from spark_common import codex_app_protocol
from spark_common.codex_runtime import build_codex_runtime_environment
from spark_common.runtime_path import resolve_runtime_workspace_path


CHAT_TURN_IDLE_TIMEOUT_SECONDS = codex_app_protocol.APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS
CONTINUITY_RESET_ERROR_CODE = "continuity_reset"
BOUNDARY_COMMAND_ENV = "SPARK_RUST_AGENT_BOUNDARY_COMMAND"
BOUNDARY_BINARY_NAME = "spark-agent-boundary"
BOUNDARY_UNAVAILABLE_MESSAGE = (
    "Rust agent boundary command is not available. Build the committed spark-agent-boundary "
    "Rust binary, install a wheel that includes spark/bin/spark-agent-boundary, or set "
    "SPARK_RUST_AGENT_BOUNDARY_COMMAND to a command that accepts a JSON payload on stdin "
    "and returns the serialized Rust adapter output."
)
SUPPORTED_AGENT_SELECTORS = {
    "codex",
    "openai",
    "anthropic",
    "gemini",
    "openrouter",
    "litellm",
    "openai_compatible",
}


def _normalize_provider(value: str | None) -> str:
    normalized = str(value or "").strip().lower()
    return normalized or "codex"


def normalize_boundary_provider_selector(value: str | None) -> str:
    normalized = _normalize_provider(value).replace("-", "_")
    if normalized == "compatible":
        normalized = "openai_compatible"
    if normalized not in SUPPORTED_AGENT_SELECTORS:
        raise ValueError(
            "Provider must be blank or one of: codex, openai, anthropic, gemini, "
            "openrouter, litellm, openai_compatible."
        )
    return normalized


def _timestamp_for_history_turn(turn: Any) -> str:
    if isinstance(turn, Mapping):
        value = turn.get("timestamp")
    else:
        value = getattr(turn, "timestamp", None)
    timestamp = as_non_empty_string(value)
    return timestamp or "1970-01-01T00:00:00Z"


def _history_value(turn: Any, key: str, default: Any = None) -> Any:
    if isinstance(turn, Mapping):
        return turn.get(key, default)
    return getattr(turn, key, default)


def rust_history_from_persisted_turns(turns: list[Any] | tuple[Any, ...]) -> list[dict[str, Any]]:
    history: list[dict[str, Any]] = []
    for turn in turns:
        role = str(_history_value(turn, "role", "") or "").strip().lower()
        status = str(_history_value(turn, "status", "complete") or "complete").strip().lower()
        kind = str(_history_value(turn, "kind", "message") or "message").strip().lower()
        content = str(_history_value(turn, "content", "") or "")
        if role not in {"user", "assistant"} or kind != "message" or status != "complete" or not content:
            continue
        history.append(
            {
                "role": role,
                "content": content,
                "timestamp": _timestamp_for_history_turn(turn),
            }
        )
    return history


def build_agent_turn_request_payload(
    *,
    conversation_id: str,
    project_path: str,
    prompt: str,
    provider: str | None,
    model: str | None,
    llm_profile: str | None,
    reasoning_effort: str | None,
    chat_mode: str | None,
    history: list[Any] | tuple[Any, ...] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "conversation_id": str(conversation_id or ""),
        "project_path": normalize_project_path_value(project_path),
        "prompt": str(prompt or ""),
        "history": rust_history_from_persisted_turns(list(history or [])),
        "provider": normalize_boundary_provider_selector(provider),
        "metadata": copy.deepcopy(dict(metadata or {})),
    }
    model_id = as_non_empty_string(model)
    profile_id = as_non_empty_string(llm_profile)
    effort = as_non_empty_string(reasoning_effort)
    mode = as_non_empty_string(chat_mode)
    if model_id is not None:
        payload["model"] = model_id
    if profile_id is not None:
        payload["llm_profile"] = profile_id
    if effort is not None:
        payload["reasoning_effort"] = effort.lower()
    if mode is not None:
        payload["chat_mode"] = mode.lower()
    return payload


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
    request_id = as_non_empty_string(
        payload.get("request_id")
        or payload.get("requestId")
        or payload.get("itemId")
        or payload.get("id")
    )
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
                question_type=as_non_empty_string(entry.get("question_type") or entry.get("questionType"))
                or _request_user_input_question_type(entry),
                options=options,
                allow_other=bool(entry.get("allow_other", entry.get("allowOther", entry.get("isOther")))),
                is_secret=bool(entry.get("is_secret", entry.get("isSecret"))),
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


def token_usage_payload_from_boundary_usage(usage: Any) -> Optional[dict[str, Any]]:
    if not isinstance(usage, Mapping):
        return None
    if isinstance(usage.get("total"), Mapping):
        return copy.deepcopy(dict(usage))
    input_tokens = int(usage.get("input_tokens", usage.get("inputTokens", 0)) or 0)
    cached_input_tokens = int(
        usage.get("cache_read_tokens", usage.get("cached_input_tokens", usage.get("cachedInputTokens", 0))) or 0
    )
    output_tokens = int(usage.get("output_tokens", usage.get("outputTokens", 0)) or 0)
    total_tokens = int(usage.get("total_tokens", usage.get("totalTokens", 0)) or 0)
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


def _turn_stream_source_from_payload(payload: Mapping[str, Any] | None) -> TurnStreamSource:
    payload = payload or {}
    summary_index = payload.get("summary_index")
    if summary_index is None:
        summary_index = payload.get("summaryIndex")
    return TurnStreamSource(
        backend=as_non_empty_string(payload.get("backend")),
        session_id=as_non_empty_string(payload.get("session_id") or payload.get("sessionId")),
        app_turn_id=as_non_empty_string(payload.get("app_turn_id") or payload.get("appTurnId")),
        item_id=as_non_empty_string(payload.get("item_id") or payload.get("itemId")),
        response_id=as_non_empty_string(payload.get("response_id") or payload.get("responseId")),
        summary_index=int(summary_index) if isinstance(summary_index, int) else None,
        raw_kind=as_non_empty_string(payload.get("raw_kind") or payload.get("rawKind")),
    )


def _source_payload_from_boundary_event(payload: Mapping[str, Any]) -> Mapping[str, Any] | None:
    source_payload = payload.get("source")
    if isinstance(source_payload, Mapping):
        return source_payload
    source_keys = {
        "backend",
        "session_id",
        "sessionId",
        "app_turn_id",
        "appTurnId",
        "item_id",
        "itemId",
        "response_id",
        "responseId",
        "summary_index",
        "summaryIndex",
        "raw_kind",
        "rawKind",
    }
    if not any(key in payload for key in source_keys):
        return None
    return {key: payload[key] for key in source_keys if key in payload}


def _default_tool_status_for_event_kind(event_kind: str) -> str:
    if event_kind == "tool_call_started":
        return "running"
    if event_kind == "tool_call_failed":
        return "failed"
    return "completed"


def _tool_record_from_boundary_payload(payload: Any, *, event_kind: str) -> Any:
    if isinstance(payload, ToolCallRecord):
        return payload
    if not isinstance(payload, Mapping):
        return payload
    normalized_payload = dict(payload)
    tool_call = _tool_call_from_item(normalized_payload)
    if tool_call is not None:
        return tool_call
    raw_paths = normalized_payload.get("file_paths")
    if raw_paths is None:
        raw_paths = normalized_payload.get("filePaths")
    item_id = as_non_empty_string(
        normalized_payload.get("id")
        or normalized_payload.get("call_id")
        or normalized_payload.get("callId")
        or normalized_payload.get("tool_call_id")
        or normalized_payload.get("toolCallId")
    )
    title = as_non_empty_string(
        normalized_payload.get("title")
        or normalized_payload.get("name")
        or normalized_payload.get("tool_name")
        or normalized_payload.get("toolName")
    )
    command = as_non_empty_string(normalized_payload.get("command"))
    output_value = normalized_payload.get("output")
    if item_id is None and title is None and command is None and output_value is None:
        return payload
    if item_id is None:
        item_id = f"tool-{uuid.uuid4().hex}"
    if title is None:
        title = "Run command" if command is not None else "Tool call"
    raw_kind_value = as_non_empty_string(normalized_payload.get("kind") or normalized_payload.get("type"))
    kind = as_non_empty_string(normalized_payload.get("tool_kind") or normalized_payload.get("toolKind"))
    if kind is None and raw_kind_value not in {
        "tool_call_started",
        "tool_call_updated",
        "tool_call_completed",
        "tool_call_failed",
    }:
        kind = raw_kind_value
    return ToolCallRecord(
        id=item_id,
        kind=kind or ("command_execution" if command is not None else "dynamic_tool"),
        status=_normalize_tool_call_status(
            normalized_payload.get("status") or _default_tool_status_for_event_kind(event_kind)
        ),
        title=title,
        command=command,
        output=str(output_value) if output_value is not None and str(output_value) else None,
        file_paths=[str(path) for path in raw_paths] if isinstance(raw_paths, list) else [],
    )


def _request_user_input_from_boundary_payload(payload: Any) -> Any:
    if isinstance(payload, RequestUserInputRecord):
        return payload
    if not isinstance(payload, dict):
        return payload
    if "request_id" in payload:
        record = RequestUserInputRecord.from_dict(payload)
        if record.request_id and record.questions:
            return record
    return _request_user_input_record_from_payload(payload) or payload


def _boundary_event_kind(payload: Mapping[str, Any]) -> str:
    return str(
        payload.get("kind")
        or payload.get("event_kind")
        or payload.get("eventKind")
        or payload.get("type")
        or ""
    )


def _boundary_content_value(payload: Mapping[str, Any]) -> Optional[str]:
    for key in ("content_delta", "contentDelta", "delta", "text", "content"):
        value = payload.get(key)
        if value is not None:
            return str(value)
    return None


def _boundary_error_message(payload: Mapping[str, Any]) -> Optional[str]:
    error = payload.get("error")
    if isinstance(error, Mapping):
        return str(error.get("message") or error.get("error") or error)
    if error is not None:
        return str(error)
    if _boundary_event_kind(payload) == "error" and payload.get("message") is not None:
        return str(payload.get("message"))
    return None


def _boundary_channel(payload: Mapping[str, Any], source: TurnStreamSource) -> Optional[str]:
    channel = as_non_empty_string(payload.get("channel"))
    if channel is not None:
        return channel
    kind = _boundary_event_kind(payload)
    if kind not in {"content_delta", "content_completed"}:
        return None
    raw_kind = str(source.raw_kind or "").lower()
    if "reasoning" in raw_kind:
        return "reasoning"
    if "plan" in raw_kind:
        return "plan"
    return "assistant"


def turn_stream_event_from_boundary_payload(payload: Any) -> TurnStreamEvent:
    if isinstance(payload, TurnStreamEvent):
        return payload
    if not isinstance(payload, Mapping):
        raise ValueError("Rust boundary event payload must be an object.")
    kind = _boundary_event_kind(payload)
    source = _turn_stream_source_from_payload(_source_payload_from_boundary_event(payload))
    token_usage = payload.get("token_usage")
    if token_usage is None:
        token_usage = payload.get("tokenUsage")
    if not isinstance(token_usage, dict):
        token_usage = token_usage_payload_from_boundary_usage(payload.get("usage"))
    tool_payload = payload.get("tool_call", payload.get("toolCall"))
    if tool_payload is None and kind in {
        "tool_call_started",
        "tool_call_updated",
        "tool_call_completed",
        "tool_call_failed",
    }:
        tool_payload = payload
    request_user_input_payload = payload.get("request_user_input", payload.get("requestUserInput"))
    if request_user_input_payload is None and kind == "request_user_input_requested":
        request_user_input_payload = payload
    return TurnStreamEvent(
        kind=kind,
        channel=_boundary_channel(payload, source),
        source=source,
        content_delta=_boundary_content_value(payload),
        message=str(payload.get("message")) if payload.get("message") is not None else None,
        tool_call=_tool_record_from_boundary_payload(tool_payload, event_kind=kind),
        request_user_input=_request_user_input_from_boundary_payload(request_user_input_payload),
        token_usage=copy.deepcopy(token_usage) if isinstance(token_usage, dict) else None,
        error=_boundary_error_message(payload),
        phase=str(payload.get("phase")) if payload.get("phase") is not None else None,
        status=str(payload.get("status")) if payload.get("status") is not None else None,
    )


class RustBoundaryError(RuntimeError):
    def __init__(
        self,
        message: str,
        *,
        retryable: bool | None = False,
        raw: Mapping[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.retryable = retryable
        self.raw = copy.deepcopy(dict(raw or {}))


class RustAgentBoundary(Protocol):
    def run_agent_turn(self, payload: dict[str, Any]) -> dict[str, Any]:
        ...

    def run_codergen(self, payload: dict[str, Any]) -> dict[str, Any]:
        ...

    def steer_codergen_turn(self, payload: dict[str, Any]) -> dict[str, Any]:
        ...


def default_rust_agent_boundary_command() -> str | None:
    packaged_binary = _packaged_rust_agent_boundary_binary_path()
    if packaged_binary is not None:
        return shlex.quote(str(packaged_binary))
    cargo_command = _source_checkout_cargo_boundary_command()
    if cargo_command is not None:
        return cargo_command
    source_binary = _source_checkout_rust_agent_boundary_binary_path()
    if source_binary is not None:
        return shlex.quote(str(source_binary))
    return None


def _platform_boundary_binary_name() -> str:
    suffix = ".exe" if os.name == "nt" else ""
    return f"{BOUNDARY_BINARY_NAME}{suffix}"


def _packaged_rust_agent_boundary_binary_path() -> Path | None:
    candidate = Path(__file__).resolve().parents[1] / "bin" / _platform_boundary_binary_name()
    return candidate if _is_runnable_file(candidate) else None


def _source_checkout_cargo_boundary_command() -> str | None:
    workspace_root = _source_checkout_root()
    if workspace_root is None or shutil.which("cargo") is None:
        return None
    manifest_path = workspace_root / "Cargo.toml"
    return " ".join(
        [
            "cargo",
            "run",
            "-q",
            "--manifest-path",
            shlex.quote(str(manifest_path)),
            "-p",
            "spark-agent-adapter",
            "--bin",
            BOUNDARY_BINARY_NAME,
            "--",
        ]
    )


def _source_checkout_rust_agent_boundary_binary_path() -> Path | None:
    workspace_root = _source_checkout_root()
    if workspace_root is None:
        return None
    binary_name = _platform_boundary_binary_name()
    for target_dir in _workspace_target_dirs(workspace_root):
        for profile in ("debug", "release"):
            candidate = target_dir / profile / binary_name
            if _is_runnable_file(candidate):
                return candidate
    return None


def _workspace_target_dirs(workspace_root: Path) -> tuple[Path, ...]:
    configured_target = os.environ.get("CARGO_TARGET_DIR", "").strip()
    target_dirs: list[Path] = []
    if configured_target:
        configured_path = Path(configured_target).expanduser()
        if not configured_path.is_absolute():
            configured_path = (Path.cwd() / configured_path).resolve(strict=False)
        target_dirs.append(configured_path)
    target_dirs.append(workspace_root / "target")
    return tuple(target_dirs)


def _source_checkout_root() -> Path | None:
    for directory in Path(__file__).resolve().parents:
        if _is_spark_rust_workspace(directory):
            return directory
    return None


def _is_spark_rust_workspace(directory: Path) -> bool:
    return (
        (directory / "Cargo.toml").is_file()
        and (directory / "pyproject.toml").is_file()
        and (directory / "crates" / "spark-agent-adapter" / "Cargo.toml").is_file()
    )


def _is_runnable_file(path: Path) -> bool:
    if not path.is_file():
        return False
    if os.name == "posix" and not os.access(path, os.X_OK):
        return False
    return True


class SerializedRustAgentBoundary:
    def __init__(self, command: str | None = None) -> None:
        configured_command = as_non_empty_string(
            command if command is not None else os.environ.get(BOUNDARY_COMMAND_ENV)
        )
        self.command = configured_command or default_rust_agent_boundary_command()

    def run_agent_turn(self, payload: dict[str, Any]) -> dict[str, Any]:
        return self._run("agent-turn", payload)

    def run_codergen(self, payload: dict[str, Any]) -> dict[str, Any]:
        timeout_seconds = payload.get("timeout_seconds")
        timeout = float(timeout_seconds) if isinstance(timeout_seconds, (int, float)) else None
        return self._run("codergen", payload, timeout_seconds=timeout)

    def steer_codergen_turn(self, payload: dict[str, Any]) -> dict[str, Any]:
        return self._run("codergen-steer", payload)

    def _run(
        self,
        operation: str,
        payload: dict[str, Any],
        *,
        timeout_seconds: float | None = None,
    ) -> dict[str, Any]:
        if self.command is None:
            raise RustBoundaryError(
                BOUNDARY_UNAVAILABLE_MESSAGE,
                retryable=False,
                raw={"kind": "rust_boundary_unconfigured", "operation": operation},
            )
        argv = [*shlex.split(self.command), operation]
        try:
            completed = subprocess.run(
                argv,
                input=json.dumps(payload, sort_keys=True),
                text=True,
                capture_output=True,
                check=False,
                timeout=timeout_seconds,
            )
        except subprocess.TimeoutExpired as exc:
            raise RustBoundaryError(
                f"Rust boundary {operation} timed out after {timeout_seconds:g}s",
                retryable=None,
                raw={"kind": "rust_boundary_timeout", "operation": operation},
            ) from exc
        if completed.returncode != 0:
            message = completed.stderr.strip() or completed.stdout.strip() or f"Rust boundary {operation} failed."
            raise RustBoundaryError(
                message,
                retryable=False,
                raw={"kind": "rust_boundary_process_failed", "operation": operation, "returncode": completed.returncode},
            )
        try:
            decoded = json.loads(completed.stdout or "{}")
        except json.JSONDecodeError as exc:
            raise RustBoundaryError(
                f"Rust boundary {operation} returned invalid JSON: {exc.msg}",
                retryable=False,
                raw={"kind": "rust_boundary_invalid_json", "operation": operation},
            ) from exc
        if not isinstance(decoded, dict):
            raise RustBoundaryError(
                f"Rust boundary {operation} returned a non-object payload.",
                retryable=False,
                raw={"kind": "rust_boundary_invalid_payload", "operation": operation},
            )
        if isinstance(decoded.get("error"), dict) and not any(key in decoded for key in ("events", "response")):
            error = decoded["error"]
            raise RustBoundaryError(
                str(error.get("message") or f"Rust boundary {operation} failed."),
                retryable=bool(error.get("retryable")),
                raw=error,
            )
        if isinstance(decoded.get("output"), dict):
            return decoded["output"]
        return decoded


def _resume_failure_summary(failure: CodexAppServerThreadResumeFailure) -> str:
    detail = "codex app-server rejected thread/resume"
    if failure.kind == "missing_thread_id":
        detail = "codex app-server returned no thread id for thread/resume"
    qualifiers: list[str] = []
    if failure.code is not None:
        qualifiers.append(f"code={failure.code}")
    if failure.message:
        qualifiers.append(f"message={failure.message}")
    if qualifiers:
        detail = f"{detail} ({', '.join(qualifiers)})"
    return detail


class PersistedThreadContinuityResetError(RuntimeError):
    """Raised when a persisted backend thread cannot be resumed safely."""

    def __init__(
        self,
        persisted_thread_id: str,
        failure: CodexAppServerThreadResumeFailure,
    ) -> None:
        self.persisted_thread_id = persisted_thread_id
        self.failure = failure
        self.error_code = CONTINUITY_RESET_ERROR_CODE
        super().__init__(
            "Continuity reset: Spark could not resume persisted thread "
            f"{persisted_thread_id!r}. A new thread was not started automatically for this turn. "
            f"Resume details: {_resume_failure_summary(failure)}."
        )

    def to_debug_payload(self) -> dict[str, Any]:
        return {
            "event": "continuity_reset",
            "error_code": self.error_code,
            "persisted_thread_id": self.persisted_thread_id,
            "replacement_thread_started": False,
            "resume_failure": self.failure.to_dict(),
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


class UnifiedAgentChatSession:
    def __init__(
        self,
        working_dir: str,
        *,
        provider: str,
        model: Optional[str] = None,
        llm_profile: Optional[str] = None,
        config_dir: Path | str | None = None,
        persisted_history: list[Any] | None = None,
        conversation_id: str = "",
        metadata: Mapping[str, Any] | None = None,
        boundary: RustAgentBoundary | None = None,
        client_factory: Any | None = None,
    ) -> None:
        del client_factory
        self.requested_working_dir = normalize_project_path_value(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.provider = normalize_boundary_provider_selector(provider)
        self.model = as_non_empty_string(model)
        self.llm_profile = as_non_empty_string(llm_profile)
        self.config_dir = Path(config_dir) if config_dir is not None else None
        self.conversation_id = str(conversation_id or "")
        self.metadata = copy.deepcopy(dict(metadata or {}))
        if self.config_dir is not None:
            self.metadata.setdefault("spark.config_dir", str(self.config_dir))
        self._persisted_history = list(persisted_history or [])
        self._boundary = boundary or SerializedRustAgentBoundary()
        self._lock = threading.Lock()
        self._raw_rpc_logger: Optional[Callable[[str, str], None]] = None
        self._pending_user_input_lock = threading.Lock()
        self._pending_user_input_by_request_id: dict[str, _PendingUserInputRequest] = {}
        self._pending_user_input_request_id_by_question_id: dict[str, str] = {}

    def close(self) -> None:
        close = getattr(self._boundary, "close", None)
        if callable(close):
            close()

    def _replace_model_unlocked(self, model: Optional[str]) -> None:
        next_model = as_non_empty_string(model)
        if next_model == self.model:
            return
        self.model = next_model

    def set_raw_rpc_logger(self, callback: Optional[Callable[[str, str], None]]) -> None:
        self._raw_rpc_logger = callback

    def clear_raw_rpc_logger(self) -> None:
        self._raw_rpc_logger = None

    def _emit_raw_log_lines(self, raw_log_lines: Any) -> None:
        if self._raw_rpc_logger is None or not isinstance(raw_log_lines, list):
            return
        for entry in raw_log_lines:
            if not isinstance(entry, Mapping):
                continue
            direction = str(entry.get("direction") or "incoming")
            line = str(entry.get("line") or "")
            if line:
                self._raw_rpc_logger(direction, line)

    def _emit_live_event(
        self,
        callback: Optional[Callable[[TurnStreamEvent], None]],
        event: TurnStreamEvent,
    ) -> None:
        if callback is not None:
            callback(event)

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

    def _handle_boundary_event(
        self,
        event_payload: Any,
        *,
        on_event: Optional[Callable[[TurnStreamEvent], None]],
    ) -> TurnStreamEvent:
        event = turn_stream_event_from_boundary_payload(event_payload)
        if event.kind == "request_user_input_requested" and isinstance(event.request_user_input, RequestUserInputRecord):
            pending = self._register_pending_user_input(event.request_user_input)
            self._emit_live_event(on_event, event)
            try:
                pending.wait_for_answers()
            finally:
                self._clear_pending_user_input(event.request_user_input.request_id)
            return event
        self._emit_live_event(on_event, event)
        return event

    def _append_turn_history(self, prompt: str, assistant_message: str) -> None:
        self._persisted_history.append(
            {
                "role": "user",
                "content": prompt,
                "timestamp": "1970-01-01T00:00:00Z",
                "status": "complete",
                "kind": "message",
            }
        )
        if assistant_message:
            self._persisted_history.append(
                {
                    "role": "assistant",
                    "content": assistant_message,
                    "timestamp": "1970-01-01T00:00:00Z",
                    "status": "complete",
                    "kind": "message",
                }
            )

    def _raise_thread_resume_failure(self, payload: Mapping[str, Any]) -> None:
        message = str(payload.get("message") or "Rust agent thread could not resume.")
        error_code = as_non_empty_string(payload.get("error_code") or payload.get("code")) or "resume_failed"
        details = payload.get("details") if isinstance(payload.get("details"), Mapping) else {}
        persisted_thread_id = as_non_empty_string(details.get("persisted_thread_id") if isinstance(details, Mapping) else None)
        persisted_thread_id = persisted_thread_id or as_non_empty_string(details.get("thread_id") if isinstance(details, Mapping) else None)
        persisted_thread_id = persisted_thread_id or ""
        raise PersistedThreadContinuityResetError(
            persisted_thread_id,
            CodexAppServerThreadResumeFailure(
                kind=error_code,
                message=message,
            ),
        )

    def _raise_boundary_output_error(self, error_payload: Any) -> None:
        if error_payload is None:
            return
        if isinstance(error_payload, Mapping):
            raise RustBoundaryError(
                str(error_payload.get("message") or error_payload.get("error") or "Rust agent boundary failed."),
                retryable=bool(error_payload.get("retryable")),
                raw=error_payload,
            )
        raise RustBoundaryError(
            str(error_payload),
            retryable=False,
            raw={"kind": "rust_boundary_error", "error": error_payload},
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
            if model is not None:
                self._replace_model_unlocked(model)
            request = build_agent_turn_request_payload(
                conversation_id=self.conversation_id,
                project_path=self.requested_working_dir,
                prompt=prompt,
                provider=self.provider,
                model=self.model,
                llm_profile=self.llm_profile,
                reasoning_effort=reasoning_effort,
                chat_mode=chat_mode,
                history=self._persisted_history,
                metadata=self.metadata,
            )
            output = self._boundary.run_agent_turn(request)
            if not isinstance(output, Mapping):
                raise RuntimeError("Rust agent boundary returned a non-object output.")
            self._emit_raw_log_lines(output.get("raw_log_lines"))
            thread_resume_failure = output.get("thread_resume_failure")
            if isinstance(thread_resume_failure, Mapping):
                self._raise_thread_resume_failure(thread_resume_failure)
            assistant_deltas: list[str] = []
            completed_assistant_text: Optional[str] = None
            completed_plan_text: Optional[str] = None
            event_token_usage: Optional[dict[str, Any]] = None
            event_error: Optional[str] = None
            raw_events = output.get("events")
            event_payloads = raw_events if isinstance(raw_events, list) else []
            for event_payload in event_payloads:
                event = self._handle_boundary_event(event_payload, on_event=on_event)
                if event.kind == "token_usage_updated" and isinstance(event.token_usage, dict):
                    event_token_usage = copy.deepcopy(event.token_usage)
                elif event.kind == "content_delta" and event.channel == "assistant" and event.content_delta:
                    assistant_deltas.append(event.content_delta)
                elif event.kind == "content_completed" and event.channel == "assistant" and event.content_delta:
                    completed_assistant_text = event.content_delta
                elif event.kind == "content_completed" and event.channel == "plan" and event.content_delta:
                    completed_plan_text = event.content_delta
                elif event.kind == "error":
                    event_error = event.error or event.message or "Rust agent boundary failed."
            self._raise_boundary_output_error(output.get("error"))
            if event_error is not None:
                raise RustBoundaryError(
                    event_error,
                    retryable=False,
                    raw={"kind": "rust_boundary_turn_error"},
                )
            output_token_usage = output.get("token_usage")
            token_usage = copy.deepcopy(output_token_usage) if isinstance(output_token_usage, dict) else None
            if token_usage is None:
                token_usage = token_usage_payload_from_boundary_usage(output.get("usage"))
            if token_usage is None and event_token_usage is not None:
                token_usage = event_token_usage
            message = str(
                output.get("final_assistant_text")
                or output.get("assistant_message")
                or output.get("text")
                or completed_assistant_text
                or completed_plan_text
                or "".join(assistant_deltas)
                or ""
            )
            self._append_turn_history(prompt, message)
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

    def _clear_thread_id(self) -> None:
        self._thread_id = None

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
            persisted_thread_id = self._thread_id
            resume_result = self._client.resume_thread(
                self._thread_id,
                model=model,
                cwd=self.working_dir,
                approval_policy="never",
            )
            if resume_result.thread_id:
                self._set_thread_id(resume_result.thread_id)
                self._thread_initialized = True
                return
            failure = resume_result.failure or CodexAppServerThreadResumeFailure(
                kind="missing_thread_id",
                message="codex app-server returned no thread id for thread/resume",
            )
            raise PersistedThreadContinuityResetError(persisted_thread_id, failure)
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
            try:
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
            except RuntimeError as exc:
                if isinstance(exc, PersistedThreadContinuityResetError):
                    self._clear_thread_id()
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
