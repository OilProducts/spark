from __future__ import annotations

from collections.abc import Mapping
from functools import partial
from pathlib import Path
from typing import Any

from unified_llm.models import get_model_info

from ..builtin_tools import edit_file as builtin_edit_file
from ..builtin_tools import glob as builtin_glob
from ..builtin_tools import grep as shared_grep
from ..builtin_tools import read_file as builtin_read_file
from ..builtin_tools import shell as builtin_shell
from ..builtin_tools import write_file as builtin_write_file
from ..subagents import register_subagent_tools
from ..tools import RegisteredTool, ToolDefinition, ToolOutput, ToolRegistry
from .base import ProviderProfile

DEFAULT_ANTHROPIC_GREP_HEAD_LIMIT = 250
DEFAULT_ANTHROPIC_SHELL_TIMEOUT_MS = 120_000
DEFAULT_ANTHROPIC_READ_FILE_LIMIT = 2000


def create_anthropic_profile(*args: Any, **kwargs: Any) -> AnthropicProviderProfile:
    return AnthropicProviderProfile(*args, **kwargs)


def _tool_result(
    content: str | dict[str, Any] | list[Any],
    *,
    is_error: bool,
) -> ToolOutput:
    return ToolOutput(content=content, is_error=is_error)


def _error(message: str) -> ToolOutput:
    return _tool_result(message, is_error=True)


def _object_tool_definition(
    name: str,
    description: str,
    *,
    properties: Mapping[str, Any],
    required: list[str],
) -> ToolDefinition:
    return ToolDefinition(
        name=name,
        description=description,
        parameters={
            "type": "object",
            "properties": dict(properties),
            "required": list(required),
            "additionalProperties": False,
        },
    )


def _anthropic_read_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "read_file",
        "Read a file and return line-numbered text or image data.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "offset": {"type": "integer", "minimum": 1, "default": 1},
            "limit": {
                "type": "integer",
                "minimum": 0,
                "default": DEFAULT_ANTHROPIC_READ_FILE_LIMIT,
            },
        },
        required=["file_path"],
    )


def _anthropic_write_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "write_file",
        "Write a file and report how many bytes were written.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "content": {"type": "string"},
        },
        required=["file_path", "content"],
    )


def _anthropic_edit_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "edit_file",
        "Edit a file by replacing exact text.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "old_string": {"type": "string", "minLength": 1},
            "new_string": {"type": "string"},
            "replace_all": {"type": "boolean", "default": False},
        },
        required=["file_path", "old_string", "new_string"],
    )


def _anthropic_shell_definition() -> ToolDefinition:
    return _object_tool_definition(
        "shell",
        "Run a shell command and return stdout, stderr, and exit metadata.",
        properties={
            "command": {"type": "string", "minLength": 1},
            "description": {"type": "string"},
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_ANTHROPIC_SHELL_TIMEOUT_MS,
            },
        },
        required=["command"],
    )


def _anthropic_glob_definition() -> ToolDefinition:
    return _object_tool_definition(
        "glob",
        "Find files matching a glob pattern.",
        properties={
            "pattern": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1},
        },
        required=["pattern"],
    )


def _anthropic_grep_definition() -> ToolDefinition:
    return _object_tool_definition(
        "grep",
        "Search files with a regex and return matching content, file paths, or per-file counts.",
        properties={
            "pattern": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1},
            "glob": {"type": "string", "minLength": 1},
            "type": {"type": "string", "minLength": 1},
            "output_mode": {
                "type": "string",
                "enum": ["content", "files_with_matches", "count"],
                "default": "files_with_matches",
            },
            "-i": {"type": "boolean", "default": False},
            "-n": {"type": "boolean", "default": True},
            "multiline": {"type": "boolean", "default": False},
            "head_limit": {
                "type": "integer",
                "minimum": 0,
                "default": DEFAULT_ANTHROPIC_GREP_HEAD_LIMIT,
            },
            "offset": {"type": "integer", "minimum": 0, "default": 0},
        },
        required=["pattern"],
    )


def _anthropic_tool_definitions() -> list[ToolDefinition]:
    return [
        _anthropic_read_file_definition(),
        _anthropic_write_file_definition(),
        _anthropic_edit_file_definition(),
        _anthropic_shell_definition(),
        _anthropic_grep_definition(),
        _anthropic_glob_definition(),
    ]


def _normalize_path_value(value: Any | None) -> str | None:
    if value is None:
        return None
    if isinstance(value, Path):
        text = str(value)
    elif isinstance(value, str):
        text = value
    else:
        return None
    if text.strip():
        return text if text.strip() else None
    return None


def _normalize_boolean(value: Any, *, default: bool) -> bool | ToolOutput:
    if value is None:
        return default
    if not isinstance(value, bool):
        return _error("boolean arguments must be booleans")
    return value


def _normalize_non_negative_int(value: Any, *, default: int) -> int | ToolOutput:
    if value is None:
        return default
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        return _error("integer arguments must be non-negative integers")
    return value


def _type_to_glob(type_name: str) -> str:
    normalized = type_name.strip()
    if not normalized:
        return normalized
    if any(character in normalized for character in "*?{[") or "/" in normalized:
        return normalized
    return f"*.{normalized.lstrip('.')}"


def _apply_window(values: list[Any], *, offset: int, head_limit: int) -> list[Any]:
    if offset > 0:
        values = values[offset:]
    if head_limit > 0:
        values = values[:head_limit]
    return values


def _match_records(content: str | dict[str, Any] | list[Any]) -> list[dict[str, Any]]:
    if isinstance(content, dict):
        matches = content.get("matches")
        if isinstance(matches, list):
            return [match for match in matches if isinstance(match, dict)]
    return []


def _match_paths(matches: list[dict[str, Any]]) -> list[str]:
    seen: set[str] = set()
    paths: list[str] = []
    for match in matches:
        path = match.get("path")
        if not isinstance(path, str) or not path:
            continue
        if path in seen:
            continue
        seen.add(path)
        paths.append(path)
    return paths


def _match_counts(matches: list[dict[str, Any]]) -> list[dict[str, Any]]:
    counts: dict[str, int] = {}
    order: list[str] = []
    for match in matches:
        path = match.get("path")
        if not isinstance(path, str) or not path:
            continue
        if path not in counts:
            order.append(path)
            counts[path] = 0
        counts[path] += 1
    return [{"path": path, "count": counts[path]} for path in order]


def grep(
    arguments: Mapping[str, Any],
    execution_environment: Any,
    provider_profile: Any | None = None,
    session_config: Any | None = None,
) -> ToolOutput:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    pattern = arguments.get("pattern")
    if not isinstance(pattern, str) or not pattern.strip():
        return _error("Missing required argument: pattern")

    path = _normalize_path_value(arguments.get("path"))
    if path is None:
        path = "."

    glob_filter = _normalize_path_value(arguments.get("glob"))
    type_name = _normalize_path_value(arguments.get("type"))
    if glob_filter is None and type_name is not None:
        glob_filter = _type_to_glob(type_name)

    output_mode = arguments.get("output_mode", "files_with_matches")
    if not isinstance(output_mode, str) or not output_mode.strip():
        return _error("output_mode must be a string")
    normalized_output_mode = output_mode.casefold()
    if normalized_output_mode not in {"content", "files_with_matches", "count"}:
        return _error("output_mode must be one of: content, files_with_matches, count")

    case_insensitive = _normalize_boolean(arguments.get("-i"), default=False)
    if isinstance(case_insensitive, ToolOutput):
        return case_insensitive

    show_line_numbers = _normalize_boolean(arguments.get("-n"), default=True)
    if isinstance(show_line_numbers, ToolOutput):
        return show_line_numbers

    multiline = _normalize_boolean(arguments.get("multiline"), default=False)
    if isinstance(multiline, ToolOutput):
        return multiline
    _ = multiline

    head_limit = _normalize_non_negative_int(
        arguments.get("head_limit"),
        default=DEFAULT_ANTHROPIC_GREP_HEAD_LIMIT,
    )
    if isinstance(head_limit, ToolOutput):
        return head_limit

    offset = _normalize_non_negative_int(arguments.get("offset"), default=0)
    if isinstance(offset, ToolOutput):
        return offset

    search_arguments: dict[str, Any] = {
        "pattern": pattern,
        "path": path,
        "case_insensitive": case_insensitive,
        "max_results": 10_000,
    }
    if glob_filter is not None:
        search_arguments["glob_filter"] = glob_filter

    base_result = shared_grep(
        search_arguments,
        execution_environment,
        provider_profile=provider_profile,
        session_config=session_config,
    )
    if base_result.is_error:
        return base_result

    matches = _match_records(base_result.content)
    if normalized_output_mode == "content":
        content_matches = _apply_window(matches, offset=offset, head_limit=head_limit)
        if not show_line_numbers:
            content_matches = [
                {key: value for key, value in match.items() if key != "line_number"}
                for match in content_matches
            ]
        return _tool_result({"matches": content_matches}, is_error=False)

    if normalized_output_mode == "files_with_matches":
        file_paths = _apply_window(_match_paths(matches), offset=offset, head_limit=head_limit)
        return _tool_result(file_paths, is_error=False)

    count_records = _apply_window(_match_counts(matches), offset=offset, head_limit=head_limit)
    return _tool_result({"files": count_records}, is_error=False)


def register_anthropic_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    executor_map = {
        "read_file": builtin_read_file,
        "write_file": builtin_write_file,
        "edit_file": builtin_edit_file,
        "shell": builtin_shell,
        "grep": grep,
        "glob": builtin_glob,
    }
    for definition in _anthropic_tool_definitions():
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
    register_subagent_tools(target_registry, provider_profile=provider_profile)
    return target_registry


def build_anthropic_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_anthropic_tools(provider_profile=provider_profile)


class AnthropicProviderProfile(ProviderProfile):
    def __post_init__(self) -> None:
        custom_tool_registry = self.tool_registry
        super().__post_init__()

        if not self.id:
            self.id = "anthropic"

        model_info = (
            get_model_info(self.model)
            if isinstance(self.model, str) and self.model
            else None
        )
        if model_info is not None:
            if self.display_name is None:
                self.display_name = model_info.display_name
            if self.context_window_size is None:
                self.context_window_size = model_info.context_window
            self.capabilities.setdefault("reasoning", model_info.supports_reasoning)
            self.capabilities.setdefault("vision", model_info.supports_vision)
            self.supports_reasoning = bool(
                self.supports_reasoning or self.capabilities.get("reasoning")
            )
        elif self.display_name is None and not self.model:
            self.display_name = "Anthropic"

        self.shell_timeout_ms = DEFAULT_ANTHROPIC_SHELL_TIMEOUT_MS
        self.tool_registry = build_anthropic_tool_registry(provider_profile=self)
        if custom_tool_registry:
            if not isinstance(custom_tool_registry, ToolRegistry):
                custom_tool_registry = ToolRegistry(custom_tool_registry)
            for name, tool in custom_tool_registry.items():
                self.tool_registry.register(tool, name=name)

    def provider_options(self, session_config: Any | None = None) -> dict[str, Any]:
        return super().provider_options(session_config)


__all__ = [
    "AnthropicProviderProfile",
    "build_anthropic_tool_registry",
    "create_anthropic_profile",
    "register_anthropic_tools",
]
