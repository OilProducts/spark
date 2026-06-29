from __future__ import annotations

from .types import SessionConfig

DEFAULT_TOOL_OUTPUT_LIMITS: dict[str, int] = {
    "apply_patch": 10_000,
    "edit_file": 10_000,
    "glob": 20_000,
    "grep": 20_000,
    "read_file": 50_000,
    "shell": 30_000,
    "spawn_agent": 20_000,
    "write_file": 1_000,
}

# Backwards-compatible alias for the spec terminology.
DEFAULT_TOOL_LIMITS = DEFAULT_TOOL_OUTPUT_LIMITS

DEFAULT_TRUNCATION_MODES: dict[str, str] = {
    "apply_patch": "tail",
    "edit_file": "tail",
    "glob": "tail",
    "grep": "tail",
    "read_file": "head_tail",
    "shell": "head_tail",
    "spawn_agent": "head_tail",
    "write_file": "tail",
}

DEFAULT_TOOL_LINE_LIMITS: dict[str, int | None] = {
    "apply_patch": None,
    "edit_file": None,
    "glob": 500,
    "grep": 200,
    "read_file": None,
    "shell": 256,
    "spawn_agent": None,
    "write_file": None,
}

# Backwards-compatible alias for the spec terminology.
DEFAULT_LINE_LIMITS = DEFAULT_TOOL_LINE_LIMITS

_HEAD_TAIL_WARNING = (
    "[WARNING: Tool output was truncated. {removed} characters were removed from the middle. "
    "The full output is available in the event stream. "
    "If you need to see specific parts, re-run the tool with more targeted parameters.]"
)
_TAIL_WARNING = (
    "[WARNING: Tool output was truncated. First {removed} characters were removed. "
    "The full output is available in the event stream.]"
)
_LINE_WARNING = (
    "[WARNING: Tool output was truncated. {omitted} lines omitted. "
    "The full output is available in the event stream.]"
)


def truncate_output(output: str, max_chars: int, mode: str) -> str:
    truncated, _ = _truncate_output_with_warning(output, max_chars, mode)
    return truncated


def _truncate_output_with_warning(
    output: str,
    max_chars: int,
    mode: str,
) -> tuple[str, str | None]:
    if max_chars < 1:
        raise ValueError("max_chars must be at least 1")
    if len(output) <= max_chars:
        return output, None

    removed = len(output) - max_chars
    if mode == "head_tail":
        head_chars = max_chars // 2
        tail_chars = max_chars - head_chars
        warning = _HEAD_TAIL_WARNING.format(removed=removed)
        return (
            output[:head_chars]
            + "\n\n"
            + warning
            + "\n\n"
            + output[-tail_chars:]
        ), warning
    if mode == "tail":
        warning = _TAIL_WARNING.format(removed=removed)
        return warning + "\n\n" + output[-max_chars:], warning
    raise ValueError(f"unsupported truncation mode: {mode}")


def truncate_lines(output: str, max_lines: int) -> str:
    if max_lines < 1:
        raise ValueError("max_lines must be at least 1")

    lines = output.split("\n")
    if len(lines) <= max_lines:
        return output

    head_count = max_lines // 2
    tail_count = max_lines - head_count
    omitted = len(lines) - head_count - tail_count
    return "\n".join(
        [*lines[:head_count], _LINE_WARNING.format(omitted=omitted), *lines[-tail_count:]]
    )


def truncate_tool_output(output: str, tool_name: str, config: SessionConfig) -> str:
    max_chars = config.tool_output_limits.get(tool_name, DEFAULT_TOOL_LIMITS.get(tool_name))
    character_warning = None
    if max_chars is not None:
        mode = DEFAULT_TRUNCATION_MODES.get(tool_name, "tail")
        output, character_warning = _truncate_output_with_warning(output, max_chars, mode)

    max_lines = config.tool_line_limits.get(tool_name, DEFAULT_LINE_LIMITS.get(tool_name))
    if max_lines is not None:
        output = truncate_lines(output, max_lines)
        if character_warning is not None and character_warning not in output:
            output = character_warning + "\n\n" + output
    return output


__all__ = [
    "DEFAULT_LINE_LIMITS",
    "DEFAULT_TOOL_LIMITS",
    "DEFAULT_TOOL_LINE_LIMITS",
    "DEFAULT_TOOL_OUTPUT_LIMITS",
    "DEFAULT_TRUNCATION_MODES",
    "truncate_lines",
    "truncate_output",
    "truncate_tool_output",
]
