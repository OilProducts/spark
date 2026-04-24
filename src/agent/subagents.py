from __future__ import annotations

import asyncio
import copy
import logging
from collections.abc import Mapping
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from typing import Any
from uuid import UUID

from unified_llm.tools import ToolResult

from .environment import ExecutionEnvironment
from .profiles.base import ProviderProfile
from .tools import RegisteredTool, ToolDefinition, ToolRegistry

logger = logging.getLogger(__name__)


def _utcnow() -> datetime:
    return datetime.now(UTC)


def _coerce_uuid(value: UUID | str | None) -> UUID | str | None:
    if not isinstance(value, str):
        return value
    try:
        return UUID(value)
    except ValueError:
        return value


def _coerce_status(value: SubAgentStatus | str) -> SubAgentStatus:
    if isinstance(value, SubAgentStatus):
        return value
    if isinstance(value, str):
        try:
            return SubAgentStatus(value)
        except ValueError:
            return SubAgentStatus[value]
    raise TypeError("status must be a SubAgentStatus or string")


def _copy_mapping(mapping: Mapping[str, Any] | None) -> dict[str, Any]:
    if mapping is None:
        return {}
    return dict(mapping)


def _clone_tool_registry(tool_registry: Any) -> Any:
    if isinstance(tool_registry, ToolRegistry):
        return ToolRegistry(dict(tool_registry.items()))
    if isinstance(tool_registry, Mapping):
        try:
            return ToolRegistry(dict(tool_registry.items()))
        except Exception:
            return tool_registry
    return copy.copy(tool_registry)


def _path_is_within(candidate: Path, root: Path) -> bool:
    try:
        candidate.relative_to(root)
    except ValueError:
        return False
    return True


def _resolve_child_scope(
    parent_working_directory: str | Path,
    working_dir: str | Path,
) -> tuple[Path, Path]:
    base_display = Path(parent_working_directory).expanduser()
    base_root = base_display.resolve(strict=False)

    child_path = Path(working_dir).expanduser()
    if child_path.is_absolute():
        scope_display = child_path
    else:
        scope_display = base_display / child_path
    scope_root = scope_display.resolve(strict=False)

    if not _path_is_within(scope_root, base_root):
        raise ValueError("working_dir must remain within the parent environment")
    return scope_display, scope_root


class AgentError(Exception):
    pass


class SubAgentError(AgentError):
    pass


class SubAgentLimitError(SubAgentError):
    pass


class SubAgentStatus(StrEnum):
    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CLOSED = "closed"


@dataclass(slots=True)
class SubAgentResult:
    handle_id: UUID | str
    status: SubAgentStatus = SubAgentStatus.COMPLETED
    session_id: UUID | str | None = None
    output: str | None = None
    success: bool | None = None
    turns_used: int | None = None
    turns: list[Any] = field(default_factory=list)
    response_id: str | None = None
    summary: str | None = None
    error: BaseException | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    timestamp: datetime = field(default_factory=_utcnow)

    def __post_init__(self) -> None:
        self.handle_id = _coerce_uuid(self.handle_id)
        self.status = _coerce_status(self.status)
        self.session_id = _coerce_uuid(self.session_id)
        self.turns = list(self.turns)
        self.metadata = dict(self.metadata)
        if self.output is None and self.summary is not None:
            self.output = self.summary
        if self.summary is None and self.output is not None:
            self.summary = self.output
        if self.success is None:
            self.success = self.status == SubAgentStatus.COMPLETED and self.error is None
        elif not isinstance(self.success, bool):
            raise TypeError("success must be a boolean or None")
        if self.turns_used is None:
            self.turns_used = len(self.turns)
        elif not isinstance(self.turns_used, int):
            raise TypeError("turns_used must be an integer or None")


@dataclass(slots=True)
class SubAgentHandle:
    id: UUID | str
    status: SubAgentStatus = SubAgentStatus.PENDING
    session: Any | None = field(default=None, repr=False, compare=False)
    session_id: UUID | str | None = None
    provider_profile: ProviderProfile | None = None
    working_directory: Path | str | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    started_at: datetime = field(default_factory=_utcnow)
    result: SubAgentResult | None = None
    task: asyncio.Task[Any] | None = field(default=None, repr=False, compare=False)

    def __post_init__(self) -> None:
        self.id = _coerce_uuid(self.id)
        self.status = _coerce_status(self.status)
        self.session_id = _coerce_uuid(self.session_id)
        if self.session is not None:
            if self.session_id is None:
                self.session_id = getattr(self.session, "id", None)
            if self.provider_profile is None:
                self.provider_profile = getattr(self.session, "provider_profile", None)
            if self.working_directory is None:
                execution_environment = getattr(self.session, "execution_environment", None)
                working_directory = getattr(execution_environment, "working_directory", None)
                if callable(working_directory):
                    self.working_directory = Path(working_directory())
        if self.working_directory is not None and not isinstance(self.working_directory, Path):
            self.working_directory = Path(self.working_directory)
        self.metadata = dict(self.metadata)
        if self.result is not None and self.status == SubAgentStatus.PENDING:
            self.status = self.result.status
        if self.task is not None and self.status == SubAgentStatus.PENDING:
            self.status = SubAgentStatus.RUNNING

    @property
    def profile(self) -> ProviderProfile | None:
        return self.provider_profile

    @profile.setter
    def profile(self, value: ProviderProfile | None) -> None:
        self.provider_profile = value

    @property
    def working_dir(self) -> Path | str | None:
        return self.working_directory

    @working_dir.setter
    def working_dir(self, value: Path | str | None) -> None:
        self.working_directory = value


@dataclass(slots=True)
class ScopedExecutionEnvironment:
    base_environment: ExecutionEnvironment
    scope_display: Path
    scope_root: Path
    delegate_prefix: Path | None = None

    def __post_init__(self) -> None:
        self.scope_display = Path(self.scope_display)
        self.scope_root = Path(self.scope_root)
        if self.delegate_prefix is not None:
            self.delegate_prefix = Path(self.delegate_prefix)
            if self.delegate_prefix == Path("."):
                self.delegate_prefix = None

    def _delegate_path(self, relative_path: Path) -> Path:
        if self.delegate_prefix is None:
            return relative_path
        return self.delegate_prefix / relative_path

    def _scope_value(self, value: str | Path) -> Path:
        candidate = Path(value).expanduser()
        if candidate.is_absolute():
            resolved = candidate.resolve(strict=False)
        else:
            resolved = (self.scope_root / candidate).resolve(strict=False)

        if not _path_is_within(resolved, self.scope_root):
            raise PermissionError(f"path escapes scoped working directory: {value}")
        relative = resolved.relative_to(self.scope_root)
        return self._delegate_path(relative)

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str | bytes:
        return self.base_environment.read_file(self._scope_value(path), offset=offset, limit=limit)

    def write_file(self, path: str | Path, content: str) -> None:
        self.base_environment.write_file(self._scope_value(path), content)

    def file_exists(self, path: str | Path) -> bool:
        return self.base_environment.file_exists(self._scope_value(path))

    def is_directory(self, path: str | Path) -> bool:
        return self.base_environment.is_directory(self._scope_value(path))

    def delete_file(self, path: str | Path) -> None:
        self.base_environment.delete_file(self._scope_value(path))

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        self.base_environment.rename_file(
            self._scope_value(source_path),
            self._scope_value(destination_path),
        )

    def list_directory(self, path: str | Path, depth: int) -> list[Any]:
        return self.base_environment.list_directory(self._scope_value(path), depth)

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: Mapping[str, str] | None = None,
    ) -> Any:
        scoped_working_dir = None if working_dir is None else self._scope_value(working_dir)
        if scoped_working_dir is None:
            scoped_working_dir = self.delegate_prefix
        return self.base_environment.exec_command(
            command,
            timeout_ms=timeout_ms,
            working_dir=scoped_working_dir,
            env_vars=env_vars,
        )

    def grep(self, pattern: str, path: str | Path, options: Any) -> str:
        return self.base_environment.grep(pattern, self._scope_value(path), options)

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        return self.base_environment.glob(pattern, self._scope_value(path))

    def initialize(self) -> None:
        self.base_environment.initialize()

    def cleanup(self) -> Any:
        return self.base_environment.cleanup()

    def working_directory(self) -> str:
        return str(self.scope_display)

    def platform(self) -> str:
        return self.base_environment.platform()

    def os_version(self) -> str:
        return self.base_environment.os_version()


def _child_environment(
    parent_environment: ExecutionEnvironment,
    working_dir: str | Path | None,
) -> ExecutionEnvironment:
    if working_dir is None:
        return parent_environment

    display_scope, resolved_scope = _resolve_child_scope(
        parent_environment.working_directory(),
        working_dir,
    )
    parent_root = Path(parent_environment.working_directory()).expanduser().resolve(strict=False)
    scope_prefix = resolved_scope.relative_to(parent_root)

    with_working_directory = getattr(parent_environment, "with_working_directory", None)
    if callable(with_working_directory):
        try:
            candidate = with_working_directory(display_scope)
        except TypeError:
            candidate = None
        else:
            if candidate is not None:
                return ScopedExecutionEnvironment(
                    base_environment=candidate,
                    scope_display=display_scope,
                    scope_root=resolved_scope,
                )

    return ScopedExecutionEnvironment(
        base_environment=parent_environment,
        scope_display=display_scope,
        scope_root=resolved_scope,
        delegate_prefix=None if scope_prefix == Path(".") else scope_prefix,
    )


def _clone_provider_profile(
    parent_profile: Any,
    *,
    model: str | None = None,
) -> ProviderProfile | Any:
    try:
        child_profile = copy.copy(parent_profile)
    except Exception:
        tool_registry = _clone_tool_registry(getattr(parent_profile, "tool_registry", None))
        child_profile = ProviderProfile(
            id=getattr(parent_profile, "id", ""),
            model=getattr(parent_profile, "model", ""),
            tool_registry=tool_registry,
            capabilities=getattr(parent_profile, "capabilities", None)
            or getattr(parent_profile, "capability_flags", {})
            or {},
            provider_options_map=getattr(parent_profile, "provider_options_map", {}) or {},
            context_window_size=getattr(parent_profile, "context_window_size", None),
            display_name=getattr(parent_profile, "display_name", None),
            knowledge_cutoff=getattr(parent_profile, "knowledge_cutoff", None),
            knowledge_cutoff_date=getattr(parent_profile, "knowledge_cutoff_date", None),
            supports_reasoning=getattr(parent_profile, "supports_reasoning", False),
            supports_streaming=getattr(parent_profile, "supports_streaming", False),
            supports_parallel_tool_calls=getattr(
                parent_profile,
                "supports_parallel_tool_calls",
                False,
            ),
        )

    if hasattr(child_profile, "tool_registry"):
        cloned_tool_registry = _clone_tool_registry(getattr(child_profile, "tool_registry", None))
        try:
            setattr(child_profile, "tool_registry", cloned_tool_registry)
        except Exception:
            pass
    for attribute_name in ("capabilities", "provider_options_map"):
        attribute_value = getattr(child_profile, attribute_name, None)
        if isinstance(attribute_value, Mapping):
            setattr(child_profile, attribute_name, dict(attribute_value))

    if model is not None:
        setattr(child_profile, "model", model)
    return child_profile


def _child_config(parent_config: Any, *, max_turns: int | None) -> Any:
    if max_turns is not None and (not isinstance(max_turns, int) or isinstance(max_turns, bool)):
        raise TypeError("max_turns must be an integer or None")
    if max_turns is not None and max_turns < 0:
        raise ValueError("max_turns must be non-negative")

    child_turn_limit = 0 if max_turns is None else max_turns
    child_depth_limit = max(int(getattr(parent_config, "max_subagent_depth", 0)) - 1, 0)

    from .types import SessionConfig

    if isinstance(parent_config, SessionConfig):
        return replace(
            parent_config,
            max_turns=child_turn_limit,
            max_subagent_depth=child_depth_limit,
        )

    try:
        return replace(
            parent_config,
            max_turns=child_turn_limit,
            max_subagent_depth=child_depth_limit,
        )
    except TypeError:
        return SessionConfig(
            max_turns=child_turn_limit,
            max_tool_rounds_per_input=getattr(parent_config, "max_tool_rounds_per_input", 0),
            default_command_timeout_ms=getattr(parent_config, "default_command_timeout_ms", 10000),
            max_command_timeout_ms=getattr(parent_config, "max_command_timeout_ms", 600000),
            reasoning_effort=getattr(parent_config, "reasoning_effort", None),
            tool_output_limits=_copy_mapping(
                getattr(parent_config, "tool_output_limits", None)
                or getattr(parent_config, "tool_output_char_limits", None)
            ),
            line_limits=_copy_mapping(
                getattr(parent_config, "line_limits", None)
                or getattr(parent_config, "tool_line_limits", None)
            ),
            enable_loop_detection=bool(getattr(parent_config, "enable_loop_detection", True)),
            loop_detection_window=int(getattr(parent_config, "loop_detection_window", 10)),
            max_subagent_depth=child_depth_limit,
        )


def _parent_session_client(parent_session: Any) -> Any:
    client = getattr(parent_session, "client", None)
    if client is not None:
        return client
    return getattr(parent_session, "llm_client", None)


def _parent_session_environment(parent_session: Any) -> ExecutionEnvironment:
    for attribute_name in ("execution_environment", "execution_env", "environment"):
        environment = getattr(parent_session, attribute_name, None)
        if environment is not None:
            return environment
    raise AttributeError("parent session has no execution environment")


def _parent_session_profile(parent_session: Any) -> Any:
    for attribute_name in ("provider_profile", "profile"):
        profile = getattr(parent_session, attribute_name, None)
        if profile is not None:
            return profile
    raise AttributeError("parent session has no provider profile")


def _parent_session_config(parent_session: Any) -> Any:
    config = getattr(parent_session, "config", None)
    if config is None:
        raise AttributeError("parent session has no config")
    return config


def _subagent_depth_limit(parent_session: Any) -> int:
    config = _parent_session_config(parent_session)
    depth = int(getattr(config, "max_subagent_depth", 0) or 0)
    return depth


def create_child_session(
    parent_session: Any,
    *,
    working_dir: str | Path | None = None,
    model: str | None = None,
    max_turns: int | None = None,
) -> SubAgentHandle:
    if _subagent_depth_limit(parent_session) <= 0:
        raise SubAgentLimitError("max_subagent_depth exceeded")

    parent_environment = _parent_session_environment(parent_session)
    child_environment = _child_environment(parent_environment, working_dir)
    child_profile = _clone_provider_profile(_parent_session_profile(parent_session), model=model)
    child_config = _child_config(_parent_session_config(parent_session), max_turns=max_turns)

    from .session import Session

    child_session = Session(
        provider_profile=child_profile,
        execution_environment=child_environment,
        client=_parent_session_client(parent_session),
        config=child_config,
    )
    child_handle = SubAgentHandle(
        id=child_session.id,
        status=SubAgentStatus.PENDING,
        session=child_session,
        session_id=child_session.id,
        provider_profile=child_profile,
        working_directory=Path(child_environment.working_directory()),
    )

    active_subagents = getattr(parent_session, "active_subagents", None)
    if isinstance(active_subagents, dict):
        active_subagents[child_handle.id] = child_handle

    return child_handle


async def close_active_subagents(parent_session: Any) -> None:
    active_subagents = getattr(parent_session, "active_subagents", None)
    if not isinstance(active_subagents, dict) or not active_subagents:
        return

    pending_handles = list(active_subagents.items())
    for handle_id, handle in pending_handles:
        active_subagents.pop(handle_id, None)
        if not isinstance(handle, SubAgentHandle):
            continue

        try:
            await _close_subagent_handle(handle)
        except Exception as exc:  # pragma: no cover - defensive cleanup
            logger.exception("failed to close subagent %s", handle_id)
            if handle.result is None:
                handle.result = SubAgentResult(
                    handle_id=handle.id,
                    status=SubAgentStatus.FAILED,
                    session_id=handle.session_id,
                    error=exc,
                )
            handle.status = handle.result.status


def _tool_result(
    content: str | dict[str, Any] | list[Any],
    *,
    is_error: bool,
) -> ToolResult:
    return ToolResult(content=content, is_error=is_error)


def _error(message: str) -> ToolResult:
    return _tool_result(message, is_error=True)


def _active_subagent_key(agent_id: UUID | str | None) -> UUID | str | None:
    return _coerce_uuid(agent_id)


def _active_subagent_handle(
    parent_session: Any,
    agent_id: UUID | str | None,
) -> SubAgentHandle | None:
    if agent_id is None:
        return None

    active_subagents = getattr(parent_session, "active_subagents", None)
    if not isinstance(active_subagents, dict):
        return None

    key = _active_subagent_key(agent_id)
    handle = active_subagents.get(key)
    if isinstance(handle, SubAgentHandle):
        return handle

    if key is not agent_id:
        handle = active_subagents.get(agent_id)
        if isinstance(handle, SubAgentHandle):
            return handle
    return None


def _subagent_response_text(session: Any) -> str:
    history = list(getattr(session, "history", []) or [])
    for turn in reversed(history):
        response_id = getattr(turn, "response_id", None)
        if isinstance(response_id, str):
            text = getattr(turn, "text", "")
            return text if isinstance(text, str) else str(text)
    return ""


def _subagent_response_id(session: Any) -> str | None:
    history = list(getattr(session, "history", []) or [])
    for turn in reversed(history):
        response_id = getattr(turn, "response_id", None)
        if isinstance(response_id, str):
            return response_id
    return None


def _subagent_result(
    handle: SubAgentHandle,
    *,
    status: SubAgentStatus,
    error: BaseException | None = None,
) -> SubAgentResult:
    session = getattr(handle, "session", None)
    history = list(getattr(session, "history", []) or [])
    output = _subagent_response_text(session)
    response_id = _subagent_response_id(session)
    return SubAgentResult(
        handle_id=handle.id,
        status=status,
        session_id=handle.session_id,
        output=output or None,
        turns=history,
        response_id=response_id,
        error=error,
        metadata=dict(handle.metadata),
    )


def _finalize_subagent_result(handle: SubAgentHandle) -> SubAgentResult | None:
    result = getattr(handle, "result", None)
    if isinstance(result, SubAgentResult):
        handle.status = result.status
        return result

    task = getattr(handle, "task", None)
    if task is None or not task.done():
        return None

    if task.cancelled():
        result = _subagent_result(handle, status=SubAgentStatus.CLOSED)
    else:
        try:
            task_exception = task.exception()
        except asyncio.CancelledError:
            result = _subagent_result(handle, status=SubAgentStatus.CLOSED)
        else:
            if task_exception is not None:
                result = _subagent_result(
                    handle,
                    status=SubAgentStatus.FAILED,
                    error=task_exception,
                )
            else:
                finalized_status = handle.status
                if finalized_status not in (
                    SubAgentStatus.COMPLETED,
                    SubAgentStatus.FAILED,
                    SubAgentStatus.CLOSED,
                ):
                    finalized_status = SubAgentStatus.COMPLETED
                result = _subagent_result(handle, status=finalized_status)

    handle.result = result
    handle.status = result.status
    return result


async def _wait_for_subagent_result(handle: SubAgentHandle) -> SubAgentResult:
    task = getattr(handle, "task", None)
    if task is not None and not task.done():
        await task

    result = _finalize_subagent_result(handle)
    if result is None:
        result = _subagent_result(handle, status=handle.status)
        handle.result = result
    return result


async def _close_subagent_handle(handle: SubAgentHandle) -> SubAgentResult:
    task = getattr(handle, "task", None)
    if task is not None and not task.done():
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            handle.status = SubAgentStatus.CLOSED
            if handle.result is None:
                handle.result = _subagent_result(handle, status=SubAgentStatus.CLOSED)
        except Exception as exc:  # pragma: no cover - defensive cleanup
            handle.status = SubAgentStatus.FAILED
            handle.result = _subagent_result(handle, status=SubAgentStatus.FAILED, error=exc)

    child_session = getattr(handle, "session", None)
    if child_session is not None:
        await child_session.close()

    result = _finalize_subagent_result(handle)
    if result is None:
        result = _subagent_result(handle, status=handle.status)
        handle.result = result
    return result


async def _spawned_child_runner(
    handle: SubAgentHandle,
    task_text: str,
) -> None:
    child_session = getattr(handle, "session", None)
    if child_session is None:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(
            handle,
            status=SubAgentStatus.FAILED,
            error=RuntimeError("child session is unavailable"),
        )
        return

    try:
        await child_session.process_input(task_text)
    except asyncio.CancelledError:
        handle.status = SubAgentStatus.CLOSED
        handle.result = _subagent_result(handle, status=SubAgentStatus.CLOSED)
    except Exception as exc:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(handle, status=SubAgentStatus.FAILED, error=exc)
    else:
        handle.status = SubAgentStatus.COMPLETED
        handle.result = _subagent_result(handle, status=SubAgentStatus.COMPLETED)


def _subagent_tool_payload(handle: SubAgentHandle) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "agent_id": str(handle.id),
        "status": handle.status.value,
    }
    if handle.session_id is not None:
        payload["session_id"] = str(handle.session_id)
    if handle.working_directory is not None:
        payload["working_dir"] = str(handle.working_directory)
    if handle.provider_profile is not None:
        model = getattr(handle.provider_profile, "model", None)
        if isinstance(model, str):
            payload["model"] = model
    return payload


def _subagent_result_payload(result: SubAgentResult) -> dict[str, Any]:
    return {
        "agent_id": str(result.handle_id),
        "session_id": str(result.session_id) if result.session_id is not None else None,
        "status": result.status.value,
        "success": result.success,
        "output": result.output,
        "turns_used": result.turns_used,
        "response_id": result.response_id,
        "summary": result.summary,
        "error": None if result.error is None else str(result.error),
        "metadata": dict(result.metadata),
    }


def spawn_agent(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    *,
    session: Any | None = None,
) -> ToolResult:
    if session is None:
        return _error("session is required")
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    task = arguments.get("task")
    if not isinstance(task, str) or not task.strip():
        return _error("Missing required argument: task")

    working_dir = arguments.get("working_dir")
    if working_dir is not None and not isinstance(working_dir, (str, Path)):
        return _error("working_dir must be a string")

    model = arguments.get("model")
    if model is not None and not isinstance(model, str):
        return _error("model must be a string")

    max_turns = arguments.get("max_turns")
    if max_turns is not None and (not isinstance(max_turns, int) or isinstance(max_turns, bool)):
        return _error("max_turns must be an integer")

    try:
        child_handle = create_child_session(
            session,
            working_dir=working_dir,
            model=model,
            max_turns=max_turns,
        )
        child_task = asyncio.create_task(_spawned_child_runner(child_handle, task))
        child_handle.task = child_task
        child_handle.status = SubAgentStatus.RUNNING
    except Exception as exc:
        if "child_handle" in locals() and isinstance(child_handle, SubAgentHandle):
            child_handle.status = SubAgentStatus.FAILED
            child_handle.result = _subagent_result(
                child_handle,
                status=SubAgentStatus.FAILED,
                error=exc,
            )
        return _error(f"Failed to spawn child agent: {exc}")

    return _tool_result(_subagent_tool_payload(child_handle), is_error=False)


def send_input(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    *,
    session: Any | None = None,
) -> ToolResult:
    if session is None:
        return _error("session is required")
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    agent_id = arguments.get("agent_id")
    if not isinstance(agent_id, (str, UUID)):
        return _error("Missing required argument: agent_id")

    message = arguments.get("message")
    if not isinstance(message, str) or not message.strip():
        return _error("Missing required argument: message")

    handle = _active_subagent_handle(session, agent_id)
    if handle is None:
        return _error(f"Unknown child agent: {agent_id}")

    finalized_result = _finalize_subagent_result(handle)
    if finalized_result is not None and finalized_result.status != SubAgentStatus.RUNNING:
        return _error(
            f"Child agent is {finalized_result.status.value}: {agent_id}"
        )

    if handle.status != SubAgentStatus.RUNNING:
        return _error(f"Child agent is {handle.status.value}: {agent_id}")

    child_task = getattr(handle, "task", None)
    if child_task is None or child_task.done():
        finalized_result = _finalize_subagent_result(handle)
        if finalized_result is None:
            return _error(f"Child agent is not running: {agent_id}")
        return _error(f"Child agent is {finalized_result.status.value}: {agent_id}")

    child_session = getattr(handle, "session", None)
    if child_session is None:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(
            handle,
            status=SubAgentStatus.FAILED,
            error=RuntimeError("child session is unavailable"),
        )
        return _error(f"Child agent is failed: {agent_id}")

    try:
        child_session.follow_up(message)
    except Exception as exc:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(handle, status=SubAgentStatus.FAILED, error=exc)
        return _error(f"Child agent is failed: {agent_id}")

    return _tool_result(_subagent_tool_payload(handle), is_error=False)


async def wait(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    *,
    session: Any | None = None,
) -> ToolResult:
    if session is None:
        return _error("session is required")
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    agent_id = arguments.get("agent_id")
    if not isinstance(agent_id, (str, UUID)):
        return _error("Missing required argument: agent_id")

    handle = _active_subagent_handle(session, agent_id)
    if handle is None:
        return _error(f"Unknown child agent: {agent_id}")

    try:
        result = await _wait_for_subagent_result(handle)
    except Exception as exc:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(handle, status=SubAgentStatus.FAILED, error=exc)
        result = handle.result

    return _tool_result(_subagent_result_payload(result), is_error=False)


async def close_agent(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    *,
    session: Any | None = None,
) -> ToolResult:
    if session is None:
        return _error("session is required")
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    agent_id = arguments.get("agent_id")
    if not isinstance(agent_id, (str, UUID)):
        return _error("Missing required argument: agent_id")

    handle = _active_subagent_handle(session, agent_id)
    if handle is None:
        return _error(f"Unknown child agent: {agent_id}")

    try:
        result = await _close_subagent_handle(handle)
    except Exception as exc:
        handle.status = SubAgentStatus.FAILED
        handle.result = _subagent_result(handle, status=SubAgentStatus.FAILED, error=exc)
        result = handle.result

    return _tool_result(_subagent_result_payload(result), is_error=False)


def subagent_tool_definitions() -> list[ToolDefinition]:
    return [
        ToolDefinition(
            name="spawn_agent",
            description="Spawn a child agent session and start it on a task.",
            parameters={
                "type": "object",
                "properties": {
                    "task": {"type": "string", "minLength": 1},
                    "working_dir": {"type": "string", "minLength": 1},
                    "model": {"type": "string"},
                    "max_turns": {"type": "integer", "minimum": 0},
                },
                "required": ["task"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="send_input",
            description="Send a follow-up message to a running child agent.",
            parameters={
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string", "minLength": 1},
                    "message": {"type": "string", "minLength": 1},
                },
                "required": ["agent_id", "message"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="wait",
            description="Wait for a child agent to finish and return its result.",
            parameters={
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string", "minLength": 1},
                },
                "required": ["agent_id"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="close_agent",
            description="Close a child agent session and cancel any running work.",
            parameters={
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string", "minLength": 1},
                },
                "required": ["agent_id"],
                "additionalProperties": False,
            },
        ),
    ]


def register_subagent_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    executor_map = {
        "spawn_agent": spawn_agent,
        "send_input": send_input,
        "wait": wait,
        "close_agent": close_agent,
    }
    for definition in subagent_tool_definitions():
        target_registry.register(
            RegisteredTool(
                definition=definition,
                executor=executor_map[definition.name],
                metadata={"kind": "subagent"},
            )
        )
    return target_registry


def build_subagent_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_subagent_tools(provider_profile=provider_profile)


__all__ = [
    "AgentError",
    "ScopedExecutionEnvironment",
    "SubAgentError",
    "SubAgentHandle",
    "SubAgentLimitError",
    "SubAgentResult",
    "SubAgentStatus",
    "close_active_subagents",
    "create_child_session",
    "build_subagent_tool_registry",
    "close_agent",
    "register_subagent_tools",
    "send_input",
    "spawn_agent",
    "subagent_tool_definitions",
    "wait",
]
