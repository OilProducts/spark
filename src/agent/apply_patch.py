from __future__ import annotations

import logging
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any

from unified_llm.tools import ToolResult

from .environment import ExecutionEnvironment
from .tools import ToolDefinition

logger = logging.getLogger(__name__)

_BEGIN_MARKER = "*** Begin Patch"
_END_MARKER = "*** End Patch"
_END_OF_FILE_MARKER = "*** End of File"

_UNICODE_PUNCTUATION_TRANSLATION = str.maketrans(
    {
        "\u00a0": " ",
        "\u2007": " ",
        "\u2009": " ",
        "\u202f": " ",
        "\u3000": " ",
        "\u2010": "-",
        "\u2011": "-",
        "\u2012": "-",
        "\u2013": "-",
        "\u2014": "-",
        "\u2015": "-",
        "\u2212": "-",
        "\u2018": "'",
        "\u2019": "'",
        "\u201a": "'",
        "\u201b": "'",
        "\u201c": '"',
        "\u201d": '"',
        "\u201e": '"',
        "\u201f": '"',
        "\u2026": "...",
    }
)


class _PatchParseError(ValueError):
    pass


class _PatchApplyError(RuntimeError):
    pass


@dataclass(slots=True)
class _PatchLine:
    kind: str
    text: str


@dataclass(slots=True)
class _PatchHunk:
    context_hint: str | None
    lines: list[_PatchLine]


@dataclass(slots=True)
class _ParsedOperation:
    kind: str
    path: str
    lines: list[str] | None = None
    move_to: str | None = None
    hunks: list[_PatchHunk] | None = None


def _tool_result(
    content: str | dict[str, Any] | list[Any],
    *,
    is_error: bool,
) -> ToolResult:
    return ToolResult(content=content, is_error=is_error)


def _error(message: str) -> ToolResult:
    return _tool_result(message, is_error=True)


def apply_patch_tool_definition() -> ToolDefinition:
    return ToolDefinition(
        name="apply_patch",
        description="Apply code changes using the v4a patch format.",
        parameters={
            "type": "object",
            "properties": {
                "patch": {"type": "string", "minLength": 1},
            },
            "required": ["patch"],
            "additionalProperties": False,
        },
    )


def _parse_patch_argument(arguments: Mapping[str, Any]) -> str:
    patch = arguments.get("patch")
    if not isinstance(patch, str) or not patch.strip():
        raise _PatchParseError("missing required argument: patch")
    return patch


def _header_path(line: str, prefix: str) -> str:
    if not line.startswith(prefix):
        raise _PatchParseError(f"expected {prefix!r}")
    path = line[len(prefix) :]
    if path.startswith(" "):
        path = path[1:]
    if not path.strip():
        raise _PatchParseError(f"missing path after {prefix!r}")
    return path


def _parse_hunk(lines: list[str], index: int) -> tuple[_PatchHunk, int]:
    header = lines[index]
    if not header.startswith("@@"):
        raise _PatchParseError("expected hunk header starting with @@")

    context_hint = header[2:]
    if context_hint.startswith(" "):
        context_hint = context_hint[1:]
    if not context_hint.strip():
        context_hint = None

    index += 1
    hunk_lines: list[_PatchLine] = []
    while index < len(lines):
        line = lines[index]
        if line.startswith("@@") or line.startswith("***"):
            break
        if not line:
            raise _PatchParseError("invalid empty hunk line")
        prefix = line[0]
        if prefix not in {" ", "-", "+"}:
            raise _PatchParseError(f"invalid hunk line: {line}")
        kind = {" ": "context", "-": "delete", "+": "add"}[prefix]
        hunk_lines.append(_PatchLine(kind=kind, text=line[1:]))
        index += 1

    if not hunk_lines:
        raise _PatchParseError("hunk must contain at least one line")
    return _PatchHunk(context_hint=context_hint, lines=hunk_lines), index


def _parse_add_operation(lines: list[str], index: int) -> tuple[_ParsedOperation, int]:
    path = _header_path(lines[index], "*** Add File:")
    index += 1
    added_lines: list[str] = []
    while index < len(lines):
        line = lines[index]
        if line.startswith("***"):
            break
        if not line.startswith("+"):
            raise _PatchParseError("add file lines must start with +")
        added_lines.append(line[1:])
        index += 1
    return _ParsedOperation(kind="add", path=path, lines=added_lines), index


def _parse_delete_operation(lines: list[str], index: int) -> tuple[_ParsedOperation, int]:
    path = _header_path(lines[index], "*** Delete File:")
    return _ParsedOperation(kind="delete", path=path), index + 1


def _parse_update_operation(lines: list[str], index: int) -> tuple[_ParsedOperation, int]:
    path = _header_path(lines[index], "*** Update File:")
    index += 1
    move_to: str | None = None
    if index < len(lines) and lines[index].startswith("*** Move to:"):
        move_to = _header_path(lines[index], "*** Move to:")
        index += 1

    hunks: list[_PatchHunk] = []
    while index < len(lines):
        line = lines[index]
        if line == _END_OF_FILE_MARKER:
            index += 1
            break
        if line.startswith("***"):
            break
        if not line.startswith("@@"):
            raise _PatchParseError("update file hunks must begin with @@")
        hunk, index = _parse_hunk(lines, index)
        hunks.append(hunk)

    if not hunks:
        raise _PatchParseError("update file must contain at least one hunk")
    return _ParsedOperation(kind="update", path=path, move_to=move_to, hunks=hunks), index


def _parse_patch_text(patch_text: str) -> list[_ParsedOperation]:
    lines = patch_text.splitlines()
    if not lines or lines[0] != _BEGIN_MARKER:
        raise _PatchParseError("missing *** Begin Patch marker")

    operations: list[_ParsedOperation] = []
    index = 1
    saw_end_marker = False
    while index < len(lines):
        line = lines[index]
        if line == _END_MARKER:
            saw_end_marker = True
            index += 1
            break
        if line.startswith("*** Add File:"):
            operation, index = _parse_add_operation(lines, index)
            operations.append(operation)
            continue
        if line.startswith("*** Delete File:"):
            operation, index = _parse_delete_operation(lines, index)
            operations.append(operation)
            continue
        if line.startswith("*** Update File:"):
            operation, index = _parse_update_operation(lines, index)
            operations.append(operation)
            continue
        if not line.strip():
            raise _PatchParseError("unexpected blank line in patch")
        raise _PatchParseError(f"unexpected patch line: {line}")

    if not saw_end_marker:
        raise _PatchParseError("missing *** End Patch marker")
    if any(line.strip() for line in lines[index:]):
        raise _PatchParseError("unexpected content after *** End Patch marker")
    return operations


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


def _is_binary_text(value: str) -> bool:
    return _contains_surrogate_escape(value) or _contains_binary_control_characters(value)


def _read_text_content(environment: ExecutionEnvironment, path: str) -> str:
    try:
        value = environment.read_file(path)
    except FileNotFoundError as exc:
        raise _PatchApplyError(f"File not found: {path}") from exc
    except PermissionError as exc:
        raise _PatchApplyError(f"Permission denied: {path}") from exc
    except IsADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except NotADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc

    if not isinstance(value, str) or _is_binary_text(value):
        raise _PatchApplyError(f"Binary file not supported: {path}")
    return value


def _split_text_content(content: str) -> tuple[list[str], str, bool]:
    if "\r\n" in content:
        separator = "\r\n"
    elif "\n" in content:
        separator = "\n"
    elif "\r" in content:
        separator = "\r"
    else:
        separator = "\n"
    return content.splitlines(), separator, bool(content) and content.endswith(("\n", "\r"))


def _join_text_content(lines: list[str], separator: str, trailing_newline: bool) -> str:
    text = separator.join(lines)
    if trailing_newline and lines:
        text += separator
    return text


def _normalize_fuzzy_text(value: str) -> str:
    normalized = unicodedata.normalize("NFKC", value)
    normalized = normalized.translate(_UNICODE_PUNCTUATION_TRANSLATION)
    return " ".join(normalized.split())


def _line_matches(candidate: str, expected: str, *, fuzzy: bool) -> bool:
    if fuzzy:
        return _normalize_fuzzy_text(candidate) == _normalize_fuzzy_text(expected)
    return candidate == expected


def _find_sequence_matches(
    haystack: list[str],
    needle: list[str],
    *,
    fuzzy: bool,
) -> list[int]:
    if not needle:
        return []
    matches: list[int] = []
    max_start = len(haystack) - len(needle)
    for start in range(max_start + 1):
        for offset, expected in enumerate(needle):
            if not _line_matches(haystack[start + offset], expected, fuzzy=fuzzy):
                break
        else:
            matches.append(start)
    return matches


def _hint_positions(haystack: list[str], hint: str) -> list[int]:
    exact_matches = [index for index, line in enumerate(haystack) if line == hint]
    if exact_matches:
        return exact_matches
    fuzzy_hint = _normalize_fuzzy_text(hint)
    return [
        index
        for index, line in enumerate(haystack)
        if _normalize_fuzzy_text(line) == fuzzy_hint
    ]


def _choose_candidate_with_hint(
    haystack: list[str],
    candidates: list[int],
    *,
    match_length: int,
    hint: str | None,
) -> int | None:
    if hint is None:
        return None

    positions = _hint_positions(haystack, hint)
    if not positions:
        return None

    best_start: int | None = None
    best_score: tuple[int, int, int] | None = None
    for start in candidates:
        end = start + match_length
        if any(start <= position < end for position in positions):
            score = (0, 0, start)
        else:
            preceding = [position for position in positions if position < start]
            if preceding:
                nearest_before = max(preceding)
                score = (1, start - nearest_before, start)
            else:
                following = [position for position in positions if position >= end]
                if following:
                    nearest_after = min(following)
                    score = (2, nearest_after - end, start)
                else:
                    score = (3, start, start)

        if best_score is None or score < best_score:
            best_score = score
            best_start = start
        elif score == best_score:
            best_start = None
    return best_start


def _apply_hunk(
    current_lines: list[str],
    hunk: _PatchHunk,
    *,
    path: str,
) -> list[str]:
    before_lines = [line.text for line in hunk.lines if line.kind != "add"]
    after_lines = [line.text for line in hunk.lines if line.kind != "delete"]
    if not before_lines:
        raise _PatchApplyError(f"Patch apply error: empty hunk in {path}")

    exact_candidates = _find_sequence_matches(current_lines, before_lines, fuzzy=False)
    if len(exact_candidates) == 1:
        start = exact_candidates[0]
    elif len(exact_candidates) > 1:
        start = _choose_candidate_with_hint(
            current_lines,
            exact_candidates,
            match_length=len(before_lines),
            hint=hunk.context_hint,
        )
        if start is None:
            raise _PatchApplyError(f"Patch apply error: ambiguous hunk match in {path}")
    else:
        fuzzy_candidates = _find_sequence_matches(current_lines, before_lines, fuzzy=True)
        if len(fuzzy_candidates) == 1:
            start = fuzzy_candidates[0]
        elif len(fuzzy_candidates) > 1:
            start = _choose_candidate_with_hint(
                current_lines,
                fuzzy_candidates,
                match_length=len(before_lines),
                hint=hunk.context_hint,
            )
            if start is None:
                raise _PatchApplyError(f"Patch apply error: ambiguous hunk match in {path}")
        else:
            raise _PatchApplyError(f"Patch apply error: unable to locate hunk in {path}")

    end = start + len(before_lines)
    return current_lines[:start] + after_lines + current_lines[end:]


def _add_file(
    environment: ExecutionEnvironment,
    operation: _ParsedOperation,
) -> dict[str, Any]:
    if operation.lines is None:
        raise _PatchApplyError("Patch apply error: malformed add file operation")

    path = operation.path
    if environment.file_exists(path):
        raise _PatchApplyError(f"File already exists: {path}")

    content = _join_text_content(operation.lines, "\n", bool(operation.lines))
    try:
        environment.write_file(path, content)
    except FileNotFoundError as exc:
        raise _PatchApplyError(f"File not found: {path}") from exc
    except PermissionError as exc:
        raise _PatchApplyError(f"Permission denied: {path}") from exc
    except IsADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except NotADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except OSError as exc:
        raise _PatchApplyError(f"Patch apply error: failed to write {path}: {exc}") from exc

    written = _read_text_content(environment, path)
    if written != content:
        raise _PatchApplyError(f"Patch verification failed: {path}")

    return {"operation": "add", "path": path}


def _delete_file(
    environment: ExecutionEnvironment,
    operation: _ParsedOperation,
) -> dict[str, Any]:
    path = operation.path
    if not environment.file_exists(path):
        raise _PatchApplyError(f"File not found: {path}")
    if environment.is_directory(path):
        raise _PatchApplyError(f"Is a directory: {path}")

    try:
        environment.delete_file(path)
    except FileNotFoundError as exc:
        raise _PatchApplyError(f"File not found: {path}") from exc
    except IsADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except PermissionError as exc:
        raise _PatchApplyError(f"Permission denied: {path}") from exc
    except FileExistsError as exc:
        raise _PatchApplyError(f"File already exists: {path}") from exc
    except OSError as exc:
        raise _PatchApplyError(f"Patch apply error: failed to delete {path}: {exc}") from exc

    if environment.file_exists(path):
        raise _PatchApplyError(f"Patch verification failed: {path}")

    return {"operation": "delete", "path": path}


def _rename_file(
    environment: ExecutionEnvironment,
    source_path_text: str,
    destination_path_text: str,
) -> None:
    if not environment.file_exists(source_path_text):
        raise _PatchApplyError(f"File not found: {source_path_text}")
    if environment.is_directory(source_path_text):
        raise _PatchApplyError(f"Is a directory: {source_path_text}")
    if environment.file_exists(destination_path_text):
        raise _PatchApplyError(f"File already exists: {destination_path_text}")

    try:
        environment.rename_file(source_path_text, destination_path_text)
    except FileNotFoundError as exc:
        raise _PatchApplyError(f"File not found: {source_path_text}") from exc
    except FileExistsError as exc:
        raise _PatchApplyError(f"File already exists: {destination_path_text}") from exc
    except IsADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {source_path_text}") from exc
    except PermissionError as exc:
        raise _PatchApplyError(f"Permission denied: {source_path_text}") from exc
    except OSError as exc:
        message = (
            "Patch apply error: failed to rename "
            f"{source_path_text} to {destination_path_text}: {exc}"
        )
        raise _PatchApplyError(message) from exc


def _update_file(
    environment: ExecutionEnvironment,
    operation: _ParsedOperation,
) -> dict[str, Any]:
    if operation.hunks is None:
        raise _PatchApplyError("Patch apply error: malformed update file operation")

    path = operation.path
    current_content = _read_text_content(environment, path)
    current_lines, separator, trailing_newline = _split_text_content(current_content)

    updated_lines = list(current_lines)
    for hunk in operation.hunks:
        updated_lines = _apply_hunk(updated_lines, hunk, path=path)

    updated_content = _join_text_content(updated_lines, separator, trailing_newline)
    try:
        environment.write_file(path, updated_content)
    except FileNotFoundError as exc:
        raise _PatchApplyError(f"File not found: {path}") from exc
    except PermissionError as exc:
        raise _PatchApplyError(f"Permission denied: {path}") from exc
    except IsADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except NotADirectoryError as exc:
        raise _PatchApplyError(f"Is a directory: {path}") from exc
    except OSError as exc:
        raise _PatchApplyError(f"Patch apply error: failed to write {path}: {exc}") from exc

    written = _read_text_content(environment, path)
    if written != updated_content:
        raise _PatchApplyError(f"Patch verification failed: {path}")

    if operation.move_to is not None and operation.move_to != path:
        _rename_file(environment, path, operation.move_to)
        renamed_content = _read_text_content(environment, operation.move_to)
        if (
            renamed_content != updated_content
            or environment.file_exists(path)
            or not environment.file_exists(operation.move_to)
        ):
            raise _PatchApplyError(f"Patch verification failed: {path} -> {operation.move_to}")
        return {
            "operation": "update+rename",
            "path": path,
            "new_path": operation.move_to,
            "hunks": len(operation.hunks),
        }

    return {"operation": "update", "path": path, "hunks": len(operation.hunks)}


def _apply_operations(
    environment: ExecutionEnvironment,
    operations: list[_ParsedOperation],
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for operation in operations:
        if operation.kind == "add":
            results.append(_add_file(environment, operation))
            continue
        if operation.kind == "delete":
            results.append(_delete_file(environment, operation))
            continue
        if operation.kind == "update":
            results.append(_update_file(environment, operation))
            continue
        raise _PatchApplyError(f"Patch apply error: unknown operation {operation.kind}")
    return results


def apply_patch(
    arguments: Mapping[str, Any],
    execution_environment: ExecutionEnvironment,
    provider_profile: Any | None = None,
) -> ToolResult:
    del provider_profile

    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    try:
        patch_text = _parse_patch_argument(arguments)
        operations = _parse_patch_text(patch_text)
        results = _apply_operations(execution_environment, operations)
    except _PatchParseError as exc:
        return _error(f"Patch parse error: {exc}")
    except _PatchApplyError as exc:
        return _error(str(exc))
    except Exception as exc:  # pragma: no cover - defensive conversion to tool error
        logger.exception("apply_patch failed")
        return _error(f"Tool error (apply_patch): {exc}")

    return _tool_result(results, is_error=False)


__all__ = ["apply_patch", "apply_patch_tool_definition"]
