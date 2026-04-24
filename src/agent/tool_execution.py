from __future__ import annotations

import asyncio
import inspect
import json
import logging
from collections.abc import Iterable, Mapping
from dataclasses import asdict
from typing import Any, Protocol

from jsonschema import SchemaError, ValidationError, validate

from unified_llm.types import ToolResultData

from .environment import ExecResult
from .events import EventKind, SessionEvent
from .truncation import truncate_tool_output
from .types import SessionConfig

logger = logging.getLogger(__name__)


class _ToolExecutionSession(Protocol):
    id: Any
    provider_profile: Any
    execution_environment: Any
    config: SessionConfig

    def emit_event(self, event: SessionEvent) -> None: ...


def _tool_value(tool_call: Any, name: str) -> Any:
    if isinstance(tool_call, Mapping):
        return tool_call[name]
    return getattr(tool_call, name)


def _tool_call_id(tool_call: Any) -> str:
    value = _tool_value(tool_call, "id")
    if not isinstance(value, str):
        raise TypeError("tool call id must be a string")
    return value


def _tool_call_name(tool_call: Any) -> str:
    value = _tool_value(tool_call, "name")
    if not isinstance(value, str):
        raise TypeError("tool call name must be a string")
    return value


def _tool_call_arguments(tool_call: Any) -> Any:
    return _tool_value(tool_call, "arguments")


def _parse_tool_arguments(arguments: Any, tool_name: str) -> dict[str, Any]:
    if arguments is None:
        return {}
    if isinstance(arguments, Mapping):
        return dict(arguments)
    if isinstance(arguments, str):
        try:
            parsed = json.loads(arguments)
        except json.JSONDecodeError as exc:
            logger.warning("Invalid JSON arguments for %s: %s", tool_name, exc)
            raise ValueError(f"Invalid arguments for tool: {tool_name}: {exc}") from exc
        if not isinstance(parsed, Mapping):
            raise ValueError(f"Invalid arguments for tool: {tool_name}: expected a JSON object")
        return dict(parsed)
    raise ValueError(f"Invalid arguments for tool: {tool_name}: expected a JSON object")


def _validate_tool_arguments(
    arguments: Mapping[str, Any],
    schema: Mapping[str, Any],
    tool_name: str,
) -> None:
    try:
        validate(instance=dict(arguments), schema=dict(schema))
    except (ValidationError, SchemaError) as exc:
        logger.warning("Validation failed for %s: %s", tool_name, exc)
        raise ValueError(f"Invalid arguments for tool: {tool_name}: {exc}") from exc


def _exec_result_details(result: ExecResult) -> dict[str, Any]:
    return asdict(result)


def _exec_result_content(result: ExecResult) -> str:
    parts: list[str] = []
    if result.stdout:
        parts.append(result.stdout)
    if result.stderr and result.stderr != result.stdout:
        parts.append(result.stderr)
    if result.exit_code != 0 or result.timed_out:
        parts.append(
            f"[exit_code={result.exit_code}, timed_out={result.timed_out}, "
            f"duration_ms={result.duration_ms}]"
        )
    return "\n".join(parts)


def _supports_parallel_tool_calls(session: _ToolExecutionSession) -> bool:
    provider_profile = getattr(session, "provider_profile", None)
    if provider_profile is None:
        return False

    supports_parallel = getattr(provider_profile, "supports_parallel_tool_calls", None)
    if isinstance(supports_parallel, bool):
        return supports_parallel
    if supports_parallel is not None and not callable(supports_parallel):
        return bool(supports_parallel)

    supports_method = getattr(provider_profile, "supports", None)
    if callable(supports_method):
        try:
            return bool(supports_method("parallel_tool_calls"))
        except TypeError:
            return bool(supports_method())

    capabilities = getattr(provider_profile, "capabilities", None)
    if isinstance(capabilities, Mapping):
        return bool(capabilities.get("parallel_tool_calls"))
    return False


def _executor_accepts_keyword_argument(executor: Any, name: str) -> bool:
    try:
        parameters = inspect.signature(executor).parameters.values()
    except (TypeError, ValueError):
        return False

    for parameter in parameters:
        if parameter.kind == inspect.Parameter.VAR_KEYWORD:
            return True
        if parameter.name == name:
            return True
    return False


def _normalize_tool_output(
    value: Any,
) -> tuple[Any, str | dict[str, Any] | list[Any], bool, bytes | None, str | None]:
    if isinstance(value, ExecResult):
        is_error = value.exit_code != 0 or value.timed_out
        if is_error:
            return _exec_result_details(value), _exec_result_content(value), True, None, None
        return value, _exec_result_content(value), False, None, None

    content_marker = getattr(value, "content", None)
    is_error_marker = getattr(value, "is_error", None)
    if content_marker is not None and isinstance(is_error_marker, bool):
        image_data = getattr(value, "image_data", None)
        image_media_type = getattr(value, "image_media_type", None)
        if image_data is not None and not isinstance(image_data, bytes):
            image_data = None
        if image_media_type is not None and not isinstance(image_media_type, str):
            image_media_type = None
        return value, content_marker, is_error_marker, image_data, image_media_type

    if isinstance(value, bytes):
        return value, value.decode("utf-8", errors="surrogateescape"), False, None, None
    if isinstance(value, (dict, list)):
        return value, value, False, None, None
    if isinstance(value, str):
        return value, value, False, None, None
    if value is None:
        return value, "", False, None, None
    return value, str(value), False, None, None


def _emit_event(session: _ToolExecutionSession, kind: EventKind, data: dict[str, Any]) -> None:
    event = SessionEvent(kind=kind, session_id=getattr(session, "id", None), data=data)
    session.emit_event(event)


def _emit_tool_call_end(
    session: _ToolExecutionSession,
    tool_call_id: str,
    tool_name: str,
    *,
    output: Any | None = None,
    error: Any | None = None,
) -> None:
    payload: dict[str, Any] = {
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
    }
    if error is not None:
        payload["error"] = error
    else:
        payload["output"] = output
    _emit_event(session, EventKind.TOOL_CALL_END, payload)


def _recoverable_tool_result(
    tool_call_id: str,
    error_message: str,
) -> ToolResultData:
    return ToolResultData(tool_call_id=tool_call_id, content=error_message, is_error=True)


async def execute_tool_call(session: _ToolExecutionSession, tool_call: Any) -> ToolResultData:
    tool_call_id = _tool_call_id(tool_call)
    tool_name = _tool_call_name(tool_call)
    _emit_event(
        session,
        EventKind.TOOL_CALL_START,
        {"tool_call_id": tool_call_id, "tool_name": tool_name},
    )

    registry = getattr(getattr(session, "provider_profile", None), "tool_registry", None)
    registered = registry.get(tool_name) if registry is not None else None
    if registered is None:
        error_msg = f"Unknown tool: {tool_name}"
        logger.warning(error_msg)
        _emit_tool_call_end(session, tool_call_id, tool_name, error=error_msg)
        return _recoverable_tool_result(tool_call_id, error_msg)

    try:
        parsed_arguments = _parse_tool_arguments(_tool_call_arguments(tool_call), tool_name)
        schema = getattr(getattr(registered, "definition", None), "parameters", {})
        if isinstance(schema, Mapping):
            _validate_tool_arguments(parsed_arguments, schema, tool_name)
        else:
            raise ValueError(f"Invalid arguments for tool: {tool_name}: schema is not a mapping")
    except ValueError as exc:
        error_msg = str(exc)
        logger.warning("Recoverable tool argument failure for %s: %s", tool_name, exc)
        _emit_tool_call_end(session, tool_call_id, tool_name, error=error_msg)
        return _recoverable_tool_result(tool_call_id, error_msg)
    except Exception as exc:
        logger.exception("Tool execution failed for %s", tool_name)
        error_msg = f"Tool error ({tool_name}): {exc}"
        _emit_tool_call_end(session, tool_call_id, tool_name, error=error_msg)
        return _recoverable_tool_result(tool_call_id, error_msg)

    executor = getattr(registered, "executor", None)
    if executor is None:
        error_msg = "tool has no executor"
        logger.warning("Recoverable tool execution failure for %s: %s", tool_name, error_msg)
        _emit_tool_call_end(session, tool_call_id, tool_name, error=error_msg)
        return _recoverable_tool_result(tool_call_id, error_msg)

    try:
        executor_kwargs: dict[str, Any] = {}
        if _executor_accepts_keyword_argument(executor, "session"):
            executor_kwargs["session"] = session
        if _executor_accepts_keyword_argument(executor, "session_config"):
            executor_kwargs["session_config"] = session.config
        raw_output = executor(
            parsed_arguments,
            getattr(session, "execution_environment", None),
            **executor_kwargs,
        )
        if inspect.isawaitable(raw_output):
            raw_output = await raw_output
    except Exception as exc:
        logger.exception("Tool execution failed for %s", tool_name)
        error_msg = f"Tool error ({tool_name}): {exc}"
        _emit_tool_call_end(session, tool_call_id, tool_name, error=error_msg)
        return _recoverable_tool_result(tool_call_id, error_msg)

    event_output, content_value, is_error, image_data, image_media_type = _normalize_tool_output(
        raw_output
    )
    if isinstance(raw_output, ExecResult) and is_error:
        logger.warning(
            "Tool execution returned failed ExecResult for %s: %s",
            tool_name,
            event_output,
        )
    if isinstance(content_value, str):
        content_value = truncate_tool_output(content_value, tool_name, session.config)

    if is_error:
        _emit_tool_call_end(session, tool_call_id, tool_name, error=event_output)
    else:
        _emit_tool_call_end(session, tool_call_id, tool_name, output=event_output)
    return ToolResultData(
        tool_call_id=tool_call_id,
        content=content_value,
        is_error=is_error,
        image_data=image_data,
        image_media_type=image_media_type,
    )


async def execute_tool_calls(
    session: _ToolExecutionSession,
    tool_calls: Iterable[Any],
) -> list[ToolResultData]:
    call_list = list(tool_calls)
    if not call_list:
        return []

    if _supports_parallel_tool_calls(session) and len(call_list) > 1:
        return list(
            await asyncio.gather(
                *(execute_tool_call(session, tool_call) for tool_call in call_list)
            )
        )

    results: list[ToolResultData] = []
    for tool_call in call_list:
        results.append(await execute_tool_call(session, tool_call))
    return results


__all__ = ["execute_tool_call", "execute_tool_calls"]
