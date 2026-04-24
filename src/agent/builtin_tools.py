from __future__ import annotations

import asyncio
import mimetypes
import re
from collections.abc import Mapping, Sequence
from functools import partial
from pathlib import Path
from typing import Any

from unified_llm.tools import ToolResult

from .apply_patch import apply_patch, apply_patch_tool_definition
from .environment import DirEntry, ExecutionEnvironment, GrepOptions
from .subagents import (
    build_subagent_tool_registry,
    register_subagent_tools,
    subagent_tool_definitions,
)
from .tools import RegisteredTool, ToolDefinition, ToolRegistry
from .types import SessionConfig

DEFAULT_READ_FILE_LIMIT = 2000


def _tool_result(
    content: str | dict[str, Any] | list[Any],
    *,
    is_error: bool,
    image_data: bytes | None = None,
    image_media_type: str | None = None,
) -> ToolResult:
    return ToolResult(
        content=content,
        is_error=is_error,
        image_data=image_data,
        image_media_type=image_media_type,
    )


def _error(message: str) -> ToolResult:
    return _tool_result(message, is_error=True)


def _path_text(value: str | Path) -> str:
    return str(value)


def _is_str_path(value: Any) -> bool:
    return isinstance(value, (str, Path))


def _is_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _contains_surrogate_escape(value: str) -> bool:
    return any(0xDC80 <= ord(character) <= 0xDCFF for character in value)


def _contains_binary_control_characters(value: str) -> bool:
    for character in value:
        codepoint = ord(character)
        if codepoint == 0:
            return True
        if codepoint < 32 and character not in {"\t", "\n", "\r"}:
            return True
        if 0x7F <= codepoint <= 0x9F:
            return True
    return False


def _maybe_binary_payload(value: str | bytes) -> bytes | None:
    if isinstance(value, bytes):
        return value
    if isinstance(value, str) and (
        _contains_surrogate_escape(value) or _contains_binary_control_characters(value)
    ):
        # Treat UTF-8-decodable payloads with embedded control bytes as binary too.
        return value.encode("utf-8", errors="surrogateescape")
    return None


def _guess_image_media_type(path: str | Path, payload: bytes) -> str | None:
    guessed_type, _ = mimetypes.guess_type(str(path))
    if isinstance(guessed_type, str) and guessed_type.startswith("image/"):
        return guessed_type

    if payload.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if payload.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if payload.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    if payload.startswith(b"BM"):
        return "image/bmp"
    if len(payload) >= 12 and payload.startswith(b"RIFF") and payload[8:12] == b"WEBP":
        return "image/webp"
    if payload.startswith((b"II*\x00", b"MM\x00*")):
        return "image/tiff"
    if payload.startswith(b"\x00\x00\x01\x00"):
        return "image/x-icon"
    return None


def _supports_multimodal(provider_profile: Any | None) -> bool:
    if provider_profile is None:
        return False

    for attribute in ("supports_multimodal", "supports_vision"):
        value = getattr(provider_profile, attribute, None)
        if isinstance(value, bool) and value:
            return True

    supports = getattr(provider_profile, "supports", None)
    if callable(supports):
        for capability in ("multimodal", "vision", "image"):
            try:
                if supports(capability):
                    return True
            except Exception:
                continue

    for attribute in ("capability_flags", "capabilities"):
        capabilities = getattr(provider_profile, attribute, None)
        if isinstance(capabilities, Mapping):
            for key in ("multimodal", "vision", "image", "supports_multimodal", "supports_vision"):
                if bool(capabilities.get(key)):
                    return True
    return False


def _provider_family(provider_profile: Any | None) -> str | None:
    if provider_profile is None:
        return None

    model = getattr(provider_profile, "model", None)
    if isinstance(model, str) and model:
        model_text = model.casefold()
        if model_text.startswith(("gpt-", "o1", "o3")):
            return "openai"

    probe_values = [
        getattr(provider_profile, "id", None),
        getattr(provider_profile, "model", None),
        getattr(provider_profile, "display_name", None),
    ]
    haystack = " ".join(value for value in probe_values if isinstance(value, str)).casefold()
    if "openai" in haystack or haystack.startswith(("gpt-", "o1", "o3")):
        return "openai"
    if "anthropic" in haystack or "claude" in haystack:
        return "anthropic"
    if "gemini" in haystack:
        return "gemini"
    return None


def _is_openai_provider(provider_profile: Any | None) -> bool:
    return _provider_family(provider_profile) == "openai"


def _format_numbered_text(text: str, *, starting_line: int = 1) -> str:
    lines = text.splitlines()
    if not lines:
        return ""
    return "\n".join(
        f"{line_number:03d} | {line}"
        for line_number, line in enumerate(lines, start=starting_line)
    )


def _read_environment_file(
    execution_environment: ExecutionEnvironment,
    path: str | Path,
    *,
    offset: int | None = None,
    limit: int | None = None,
) -> str | bytes:
    return execution_environment.read_file(path, offset=offset, limit=limit)


def _read_text_or_binary_file(
    execution_environment: ExecutionEnvironment,
    path: str | Path,
    *,
    offset: int | None = None,
    limit: int | None = None,
) -> tuple[str | bytes, str | None]:
    try:
        value = _read_environment_file(
            execution_environment,
            path,
            offset=offset,
            limit=limit,
        )
    except FileNotFoundError as exc:
        raise FileNotFoundError(_path_text(path)) from exc
    except PermissionError as exc:
        raise PermissionError(_path_text(path)) from exc
    except IsADirectoryError as exc:
        raise IsADirectoryError(_path_text(path)) from exc
    except NotADirectoryError as exc:
        raise NotADirectoryError(_path_text(path)) from exc
    except UnicodeDecodeError as exc:
        raw_object = exc.object
        if isinstance(raw_object, bytes):
            payload = raw_object
        elif isinstance(raw_object, str):
            payload = raw_object.encode("utf-8", errors="surrogateescape")
        else:
            payload = None
        if payload is None:
            raise
        return payload, _guess_image_media_type(path, payload)
    payload = _maybe_binary_payload(value)
    if payload is None:
        return value, None
    return payload, _guess_image_media_type(path, payload)


def _read_text_file_content(
    execution_environment: ExecutionEnvironment,
    path: str | Path,
    *,
    offset: int = 1,
    limit: int = DEFAULT_READ_FILE_LIMIT,
) -> str | ToolResult:
    try:
        value = _read_environment_file(
            execution_environment,
            path,
            offset=offset,
            limit=limit,
        )
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except UnicodeDecodeError:
        return _error(f"Binary file not supported: {_path_text(path)}")

    payload = _maybe_binary_payload(value)
    if payload is not None:
        return _error(f"Binary file not supported: {_path_text(path)}")

    if not isinstance(value, str):
        return _error(f"Binary file not supported: {_path_text(path)}")
    if _contains_surrogate_escape(value):
        return _error(f"Binary file not supported: {_path_text(path)}")
    return _format_numbered_text(value, starting_line=offset)


def _parse_path_argument(arguments: Mapping[str, Any], *names: str) -> str | Path | None:
    for name in names:
        if name not in arguments:
            continue
        value = arguments[name]
        if _is_str_path(value):
            text = _path_text(value)
            if not text.strip():
                return None
            return value
        return None
    return None


def _parse_required_string_argument(
    arguments: Mapping[str, Any],
    *names: str,
) -> str | None:
    for name in names:
        if name not in arguments:
            continue
        value = arguments[name]
        if isinstance(value, str):
            return value
        return None
    return None


def _parse_int_argument(
    arguments: Mapping[str, Any],
    name: str,
    *,
    default: int,
    minimum: int,
) -> int | None:
    if name not in arguments:
        return default
    value = arguments[name]
    if not _is_int(value) or value < minimum:
        return None
    return value


def _parse_bool_argument(
    arguments: Mapping[str, Any],
    name: str,
    *,
    default: bool,
) -> bool | None:
    if name not in arguments:
        return default
    value = arguments[name]
    if not isinstance(value, bool):
        return None
    return value


def _parse_optional_int_argument(
    arguments: Mapping[str, Any],
    name: str,
    *,
    minimum: int,
) -> int | None:
    if name not in arguments:
        return None
    value = arguments[name]
    if not _is_int(value) or value < minimum:
        return None
    return value


def _default_search_path(execution_environment: ExecutionEnvironment) -> str | Path:
    # Search from the environment's active directory instead of echoing a
    # configured relative working-directory string back into the backend.
    return "."


def _provider_options_mapping(provider_profile: Any | None) -> Mapping[str, Any] | None:
    if provider_profile is None:
        return None

    provider_options = getattr(provider_profile, "provider_options_map", None)
    if isinstance(provider_options, Mapping):
        return provider_options

    provider_options_method = getattr(provider_profile, "provider_options", None)
    if callable(provider_options_method):
        try:
            provider_options = provider_options_method()
        except Exception:
            return None
        if isinstance(provider_options, Mapping):
            return provider_options

    return None


def _coerce_positive_int(value: Any) -> int | None:
    if _is_int(value) and value >= 1:
        return value
    return None


def _select_provider_default_timeout_ms(
    provider_profile: Any | None,
    *,
    tool_name: str,
) -> int | None:
    provider_options = _provider_options_mapping(provider_profile)
    if provider_options is not None:
        tool_options = provider_options.get(tool_name)
        if isinstance(tool_options, Mapping):
            for candidate_name in ("timeout_ms", "default_timeout_ms", "command_timeout_ms"):
                timeout_ms = _coerce_positive_int(tool_options.get(candidate_name))
                if timeout_ms is not None:
                    return timeout_ms

        for candidate_name in (
            f"{tool_name}_timeout_ms",
            "command_timeout_ms",
            "default_command_timeout_ms",
        ):
            timeout_ms = _coerce_positive_int(provider_options.get(candidate_name))
            if timeout_ms is not None:
                return timeout_ms

        tool_timeouts = provider_options.get("tool_timeouts")
        if isinstance(tool_timeouts, Mapping):
            timeout_ms = _coerce_positive_int(tool_timeouts.get(tool_name))
            if timeout_ms is not None:
                return timeout_ms

    if provider_profile is None:
        return None

    for attribute_name in (
        f"{tool_name}_timeout_ms",
        "default_command_timeout_ms",
        "default_timeout_ms",
        "command_timeout_ms",
    ):
        candidate = getattr(provider_profile, attribute_name, None)
        timeout_ms = _coerce_positive_int(candidate)
        if timeout_ms is not None:
            return timeout_ms

        if callable(candidate):
            for call_args in ((), (tool_name,)):
                try:
                    selected = candidate(*call_args)
                except TypeError:
                    continue
                except Exception:
                    break
                timeout_ms = _coerce_positive_int(selected)
                if timeout_ms is not None:
                    return timeout_ms
                break

    for attribute_name in (
        "get_default_command_timeout_ms",
        "get_default_timeout_ms",
        "select_command_timeout_ms",
        "select_timeout_ms",
    ):
        candidate = getattr(provider_profile, attribute_name, None)
        if not callable(candidate):
            continue
        for call_args in ((), (tool_name,)):
            try:
                selected = candidate(*call_args)
            except TypeError:
                continue
            except Exception:
                break
            timeout_ms = _coerce_positive_int(selected)
            if timeout_ms is not None:
                return timeout_ms
            break

    return None


def _resolve_shell_timeout_ms(
    arguments: Mapping[str, Any],
    *,
    provider_profile: Any | None,
    session_config: SessionConfig | None,
) -> int | None | ToolResult:
    explicit_timeout_ms = _parse_optional_int_argument(arguments, "timeout_ms", minimum=1)
    if "timeout_ms" in arguments and explicit_timeout_ms is None:
        return _error("timeout_ms must be at least 1")

    timeout_ms = explicit_timeout_ms
    if timeout_ms is None:
        timeout_ms = _select_provider_default_timeout_ms(provider_profile, tool_name="shell")

    if timeout_ms is None and session_config is not None:
        timeout_ms = _coerce_positive_int(session_config.default_command_timeout_ms)

    if timeout_ms is not None and session_config is not None:
        max_timeout_ms = _coerce_positive_int(session_config.max_command_timeout_ms)
        if max_timeout_ms is not None:
            timeout_ms = min(timeout_ms, max_timeout_ms)

    return timeout_ms


def _parse_grep_matches(output: str) -> list[dict[str, Any]]:
    matches: list[dict[str, Any]] = []
    for line in output.splitlines():
        if not line.strip():
            continue
        match = re.match(r"^(.+):(\d+):(.*)$", line)
        if match is None:
            continue
        path_text, line_number_text, line_text = match.groups()
        line_number = int(line_number_text)
        matches.append(
            {
                "path": path_text,
                "line_number": line_number,
                "line": line_text.rstrip("\r"),
            }
        )
    return matches


async def shell(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
    session_config: SessionConfig | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    command = _parse_required_string_argument(arguments, "command")
    if command is None or not command.strip():
        return _error("Missing required argument: command")

    timeout_ms = _resolve_shell_timeout_ms(
        arguments,
        provider_profile=provider_profile,
        session_config=session_config,
    )
    if isinstance(timeout_ms, ToolResult):
        return timeout_ms

    try:
        result = await asyncio.to_thread(
            execution_environment.exec_command,
            command,
            timeout_ms=timeout_ms,
        )
    except ValueError as exc:
        return _error(str(exc))
    except FileNotFoundError:
        return _error(f"File not found: {command}")
    except PermissionError:
        return _error(f"Permission denied: {command}")
    except IsADirectoryError:
        return _error(f"Is a directory: {command}")
    except NotADirectoryError:
        return _error(f"Is a directory: {command}")
    except OSError as exc:
        return _error(f"Failed to execute command: {exc}")

    stdout = getattr(result, "stdout", "") or ""
    stderr = getattr(result, "stderr", "") or ""
    exit_code = getattr(result, "exit_code", 0)
    timed_out = bool(getattr(result, "timed_out", False))
    duration_ms = getattr(result, "duration_ms", 0)

    payload = {
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "duration_ms": duration_ms,
    }
    return _tool_result(payload, is_error=bool(exit_code != 0 or timed_out))


def grep(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
    session_config: SessionConfig | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    pattern = _parse_required_string_argument(arguments, "pattern")
    if pattern is None or not pattern.strip():
        return _error("Missing required argument: pattern")

    path_value = arguments.get("path")
    if path_value is None:
        path = _default_search_path(execution_environment)
    else:
        path = _parse_path_argument(arguments, "path")
        if path is None:
            return _error("Missing required argument: path")

    glob_filter = arguments.get("glob_filter")
    if glob_filter is None:
        glob_filter_text = None
    elif isinstance(glob_filter, str) and glob_filter.strip():
        glob_filter_text = glob_filter
    else:
        return _error("glob_filter must be a string")

    case_insensitive = _parse_bool_argument(
        arguments,
        "case_insensitive",
        default=False,
    )
    if case_insensitive is None:
        return _error("case_insensitive must be a boolean")

    max_results = _parse_int_argument(arguments, "max_results", default=100, minimum=1)
    if max_results is None:
        return _error("max_results must be at least 1")

    options = GrepOptions(
        glob_filter=glob_filter_text,
        case_insensitive=case_insensitive,
        max_results=max_results,
    )
    try:
        output = execution_environment.grep(pattern, path, options)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except ValueError as exc:
        return _error(f"Invalid regex pattern: {exc}")

    return _tool_result({"matches": _parse_grep_matches(output)}, is_error=False)


def glob(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
    session_config: SessionConfig | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    pattern = _parse_required_string_argument(arguments, "pattern")
    if pattern is None or not pattern.strip():
        return _error("Missing required argument: pattern")

    path_value = arguments.get("path")
    if path_value is None:
        path = _default_search_path(execution_environment)
    else:
        path = _parse_path_argument(arguments, "path")
        if path is None:
            return _error("Missing required argument: path")

    try:
        matches = execution_environment.glob(pattern, path)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except ValueError as exc:
        return _error(f"Invalid glob pattern: {exc}")

    return _tool_result(
        [
            _path_text(candidate) if _is_str_path(candidate) else str(candidate)
            for candidate in matches
        ],
        is_error=False,
    )


def read_file(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    path = _parse_path_argument(arguments, "path", "file_path")
    if path is None:
        return _error("Missing required argument: path")

    offset = _parse_int_argument(arguments, "offset", default=1, minimum=1)
    if offset is None:
        return _error("offset must be at least 1")

    limit = _parse_int_argument(
        arguments,
        "limit",
        default=DEFAULT_READ_FILE_LIMIT,
        minimum=0,
    )
    if limit is None:
        return _error("limit must be non-negative")

    try:
        value, media_type = _read_text_or_binary_file(
            execution_environment,
            path,
            offset=offset,
            limit=limit,
        )
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")

    if isinstance(value, str) and media_type is None:
        return _tool_result(
            _format_numbered_text(value, starting_line=offset),
            is_error=False,
        )

    if media_type is None:
        return _error(f"Binary file not supported: {_path_text(path)}")

    if not _supports_multimodal(provider_profile):
        return _error(f"Binary file not supported: {_path_text(path)}")

    try:
        full_value = _read_environment_file(execution_environment, path)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except UnicodeDecodeError:
        return _error(f"Binary file not supported: {_path_text(path)}")

    full_payload = _maybe_binary_payload(full_value)
    if full_payload is None:
        return _error(f"Binary file not supported: {_path_text(path)}")

    detected_media_type = _guess_image_media_type(path, full_payload) or media_type
    if detected_media_type is None:
        return _error(f"Binary file not supported: {_path_text(path)}")

    content = f"{_path_text(path)} [{detected_media_type}, {len(full_payload)} bytes]"
    return _tool_result(
        content,
        is_error=False,
        image_data=full_payload,
        image_media_type=detected_media_type,
    )


def write_file(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    path = _parse_path_argument(arguments, "path", "file_path")
    if path is None:
        return _error("Missing required argument: path")

    content = _parse_required_string_argument(arguments, "content", "text")
    if content is None:
        return _error("Missing required argument: content")

    bytes_written = len(content.encode("utf-8", errors="surrogateescape"))
    try:
        execution_environment.write_file(path, content)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except OSError as exc:
        return _error(f"Failed to write file: {_path_text(path)}: {exc}")

    return _tool_result(
        {"path": _path_text(path), "bytes_written": bytes_written},
        is_error=False,
    )


def edit_file(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    path = _parse_path_argument(arguments, "path", "file_path")
    if path is None:
        return _error("Missing required argument: path")

    old_string = _parse_required_string_argument(arguments, "old_string", "old")
    if old_string is None:
        return _error("Missing required argument: old_string")
    if old_string == "":
        return _error("old_string must not be empty")

    new_string = _parse_required_string_argument(arguments, "new_string", "new")
    if new_string is None:
        return _error("Missing required argument: new_string")

    replace_all = _parse_bool_argument(arguments, "replace_all", default=False)
    if replace_all is None:
        return _error("replace_all must be a boolean")

    try:
        current_value = _read_environment_file(execution_environment, path)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except UnicodeDecodeError:
        return _error(f"Binary file not supported: {_path_text(path)}")

    payload = _maybe_binary_payload(current_value)
    if payload is not None or not isinstance(current_value, str):
        return _error(f"Binary file not supported: {_path_text(path)}")
    if _contains_surrogate_escape(current_value):
        return _error(f"Binary file not supported: {_path_text(path)}")

    occurrences = current_value.count(old_string)
    if occurrences == 0:
        return _error(f"old_string not found in {_path_text(path)}")
    if not replace_all and occurrences != 1:
        return _error(
            f"old_string is not unique in {_path_text(path)}: {occurrences} matches"
        )

    replacement_count = occurrences if replace_all else 1
    updated_value = current_value.replace(
        old_string,
        new_string,
        -1 if replace_all else 1,
    )
    bytes_written = len(updated_value.encode("utf-8", errors="surrogateescape"))
    try:
        execution_environment.write_file(path, updated_value)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except OSError as exc:
        return _error(f"Failed to write file: {_path_text(path)}: {exc}")

    return _tool_result(
        {
            "path": _path_text(path),
            "replacements": replacement_count,
            "bytes_written": bytes_written,
            "replace_all": replace_all,
        },
        is_error=False,
    )


def read_many_files(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    paths_value = arguments.get("paths", arguments.get("files", arguments.get("file_paths")))
    if paths_value is None:
        return _error("Missing required argument: paths")
    if _is_str_path(paths_value):
        path_values: list[str | Path] = [paths_value]
    elif isinstance(paths_value, Sequence) and not isinstance(
        paths_value,
        (str, bytes, bytearray, memoryview),
    ):
        path_values = []
        for value in paths_value:
            if not _is_str_path(value):
                return _error("paths must be strings or paths")
            path_values.append(value)
    else:
        return _error("paths must be a list of strings or paths")

    if not path_values:
        return _error("paths must not be empty")

    file_records: list[dict[str, Any]] = []
    for path in path_values:
        content = _read_text_file_content(
            execution_environment,
            path,
            offset=1,
            limit=DEFAULT_READ_FILE_LIMIT,
        )
        if isinstance(content, ToolResult):
            return content
        file_records.append(
            {
                "path": _path_text(path),
                "content": content,
            }
        )

    return _tool_result(
        {"count": len(file_records), "files": file_records},
        is_error=False,
    )


def list_dir(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    path = _parse_path_argument(arguments, "path", "directory")
    if path is None:
        return _error("Missing required argument: path")

    depth = _parse_int_argument(arguments, "depth", default=0, minimum=0)
    if depth is None:
        return _error("depth must be non-negative")

    try:
        entries = execution_environment.list_directory(path, depth)
    except FileNotFoundError:
        return _error(f"File not found: {_path_text(path)}")
    except PermissionError:
        return _error(f"Permission denied: {_path_text(path)}")
    except IsADirectoryError:
        return _error(f"Is a directory: {_path_text(path)}")
    except NotADirectoryError:
        return _error(f"Not a directory: {_path_text(path)}")

    entry_records: list[dict[str, Any]] = []
    for entry in entries:
        if not isinstance(entry, DirEntry):
            entry_records.append(
                {
                    "name": str(getattr(entry, "name", "")),
                    "is_dir": bool(getattr(entry, "is_dir", False)),
                    "size": getattr(entry, "size", None),
                }
            )
            continue
        entry_records.append(
            {
                "name": entry.name,
                "is_dir": entry.is_dir,
                "size": entry.size,
            }
        )

    return _tool_result(
        {
            "path": _path_text(path),
            "depth": depth,
            "count": len(entry_records),
            "entries": entry_records,
        },
        is_error=False,
    )


def builtin_tool_definitions(provider_profile: Any | None = None) -> list[ToolDefinition]:
    definitions = [
        ToolDefinition(
            name="read_file",
            description="Read a file and return numbered text lines or image data.",
            parameters={
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "offset": {"type": "integer", "minimum": 1, "default": 1},
                    "limit": {"type": "integer", "minimum": 0, "default": DEFAULT_READ_FILE_LIMIT},
                },
                "required": ["path"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="write_file",
            description="Write a file and report how many bytes were written.",
            parameters={
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "content": {"type": "string"},
                },
                "required": ["path", "content"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="edit_file",
            description="Edit a file by replacing exact text.",
            parameters={
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "old_string": {"type": "string", "minLength": 1},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean", "default": False},
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="shell",
            description="Run a shell command and return stdout, stderr, and exit metadata.",
            parameters={
                "type": "object",
                "properties": {
                    "command": {"type": "string", "minLength": 1},
                    "timeout_ms": {"type": "integer", "minimum": 1},
                    "description": {"type": "string"},
                },
                "required": ["command"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="grep",
            description="Search files with a regex and return structured matches.",
            parameters={
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "minLength": 1},
                    "glob_filter": {"type": "string", "minLength": 1},
                    "case_insensitive": {"type": "boolean", "default": False},
                    "max_results": {"type": "integer", "minimum": 1, "default": 100},
                },
                "required": ["pattern"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="glob",
            description="Expand a glob pattern and return matching paths.",
            parameters={
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "minLength": 1},
                },
                "required": ["pattern"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="read_many_files",
            description="Read several files and return numbered text for each one.",
            parameters={
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {"type": "string", "minLength": 1},
                        "minItems": 1,
                    },
                },
                "required": ["paths"],
                "additionalProperties": False,
            },
        ),
        ToolDefinition(
            name="list_dir",
            description="List a directory and return structured entries.",
            parameters={
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "depth": {"type": "integer", "minimum": 0, "default": 0},
                },
                "required": ["path"],
                "additionalProperties": False,
            },
        ),
    ]
    if _is_openai_provider(provider_profile):
        definitions.append(apply_patch_tool_definition())
    return definitions


_FILE_TOOL_NAMES: tuple[str, ...] = (
    "read_file",
    "write_file",
    "edit_file",
    "read_many_files",
    "list_dir",
)


def builtin_file_tool_definitions() -> list[ToolDefinition]:
    builtin_definitions = builtin_tool_definitions()
    builtin_definition_map = {definition.name: definition for definition in builtin_definitions}
    return [builtin_definition_map[name] for name in _FILE_TOOL_NAMES]


_OPENAI_SHELL_DEFAULT_TIMEOUT_MS = 10_000


def _openai_shell(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    *,
    provider_profile: Any | None = None,
    session_config: SessionConfig | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    normalized_arguments = dict(arguments)
    normalized_arguments.setdefault("timeout_ms", _OPENAI_SHELL_DEFAULT_TIMEOUT_MS)
    return shell(
        normalized_arguments,
        execution_environment,
        provider_profile=provider_profile,
        session_config=session_config,
    )


def _openai_shell_tool_definition(definition: ToolDefinition) -> ToolDefinition:
    parameters = dict(definition.parameters)
    properties = dict(parameters.get("properties", {}))
    timeout_schema = dict(properties.get("timeout_ms", {}))
    timeout_schema["default"] = _OPENAI_SHELL_DEFAULT_TIMEOUT_MS
    properties["timeout_ms"] = timeout_schema
    parameters["properties"] = properties
    return ToolDefinition(
        name=definition.name,
        description=definition.description,
        parameters=parameters,
        metadata=dict(definition.metadata),
    )


def openai_builtin_tool_definitions() -> list[ToolDefinition]:
    builtin_definitions = builtin_tool_definitions()
    builtin_definition_map = {definition.name: definition for definition in builtin_definitions}
    return [
        builtin_definition_map["read_file"],
        apply_patch_tool_definition(),
        builtin_definition_map["write_file"],
        _openai_shell_tool_definition(builtin_definition_map["shell"]),
        builtin_definition_map["grep"],
        builtin_definition_map["glob"],
    ]


def register_openai_builtin_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    executor_map = {
        "read_file": read_file,
        "apply_patch": apply_patch,
        "write_file": write_file,
        "shell": _openai_shell,
        "grep": grep,
        "glob": glob,
    }
    for definition in openai_builtin_tool_definitions():
        target_registry.register(
            RegisteredTool(
                definition=definition,
                executor=partial(
                    executor_map[definition.name],
                    provider_profile=provider_profile,
                ),
                metadata={"kind": "builtin"},
            )
        )
    return target_registry


def build_openai_builtin_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_openai_builtin_tools(provider_profile=provider_profile)


def _builtin_executor_for_definition(
    definition_name: str,
    *,
    provider_profile: Any | None,
) -> Any:
    executor_map = {
        "read_file": read_file,
        "write_file": write_file,
        "edit_file": edit_file,
        "shell": shell,
        "grep": grep,
        "glob": glob,
        "read_many_files": read_many_files,
        "list_dir": list_dir,
        "apply_patch": apply_patch,
    }
    executor = executor_map.get(definition_name)
    if executor is None:
        raise KeyError(f"unknown builtin tool: {definition_name}")
    return partial(executor, provider_profile=provider_profile)


def _register_tool_definitions(
    registry: ToolRegistry | None,
    definitions: list[ToolDefinition],
    *,
    provider_profile: Any | None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    for definition in definitions:
        target_registry.register(
            RegisteredTool(
                definition=definition,
                executor=_builtin_executor_for_definition(
                    definition.name,
                    provider_profile=provider_profile,
                ),
                metadata={"kind": "builtin"},
            )
        )
    return target_registry


def register_builtin_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return _register_tool_definitions(
        registry,
        builtin_tool_definitions(provider_profile=provider_profile),
        provider_profile=provider_profile,
    )


def build_builtin_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_builtin_tools(provider_profile=provider_profile)


def register_builtin_file_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return _register_tool_definitions(
        registry,
        builtin_file_tool_definitions(),
        provider_profile=provider_profile,
    )


def build_builtin_file_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_builtin_file_tools(provider_profile=provider_profile)


def register_file_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_builtin_file_tools(registry, provider_profile=provider_profile)


def build_file_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_builtin_file_tools(provider_profile=provider_profile)


def file_tool_definitions() -> list[ToolDefinition]:
    return builtin_file_tool_definitions()


__all__ = [
    "DEFAULT_READ_FILE_LIMIT",
    "apply_patch",
    "build_builtin_file_tool_registry",
    "build_builtin_tool_registry",
    "build_openai_builtin_tool_registry",
    "build_subagent_tool_registry",
    "build_file_tool_registry",
    "builtin_file_tool_definitions",
    "builtin_tool_definitions",
    "edit_file",
    "glob",
    "grep",
    "file_tool_definitions",
    "list_dir",
    "openai_builtin_tool_definitions",
    "read_file",
    "read_many_files",
    "register_openai_builtin_tools",
    "shell",
    "register_file_tools",
    "register_builtin_file_tools",
    "register_builtin_tools",
    "register_subagent_tools",
    "subagent_tool_definitions",
    "write_file",
]
