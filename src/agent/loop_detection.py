from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from typing import Any

LOOP_DETECTION_WARNING = (
    "Repeated tool-call loop detected. Try a different approach instead of "
    "repeating the same tool calls."
)


@dataclass(frozen=True, slots=True)
class ToolCallSignature:
    name: str
    arguments_hash: str


_MISSING = object()


def _tool_call_field(tool_call: Any, field_name: str, default: Any = _MISSING) -> Any:
    if isinstance(tool_call, Mapping):
        return tool_call.get(field_name, default)
    return getattr(tool_call, field_name, default)


def _tool_call_name(tool_call: Any) -> str:
    name = _tool_call_field(tool_call, "name")
    if not isinstance(name, str):
        raise TypeError("tool call name must be a string")
    return name


def _normalized_mapping_key(key: Any) -> str:
    if isinstance(key, str):
        return key
    return f"{type(key).__name__}:{key!r}"


def _canonicalize_argument_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        normalized_items = sorted(
            (
                (_normalized_mapping_key(key), _canonicalize_argument_value(item))
                for key, item in value.items()
            ),
            key=lambda item: item[0],
        )
        return {key: item for key, item in normalized_items}
    if isinstance(value, (list, tuple)):
        return [_canonicalize_argument_value(item) for item in value]
    if isinstance(value, (set, frozenset)):
        normalized_items = [_canonicalize_argument_value(item) for item in value]
        return sorted(
            normalized_items,
            key=lambda item: json.dumps(
                item,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            ),
        )
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="surrogateescape")
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    return str(value)


def _canonicalize_arguments(arguments: Any) -> Any:
    if isinstance(arguments, str):
        try:
            parsed = json.loads(arguments)
        except json.JSONDecodeError:
            return arguments
        return _canonicalize_argument_value(parsed)
    return _canonicalize_argument_value(arguments)


def _arguments_hash(arguments: Any) -> str:
    canonical_arguments = _canonicalize_arguments(arguments)
    canonical_json = json.dumps(
        canonical_arguments,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    )
    return hashlib.sha256(canonical_json.encode("utf-8")).hexdigest()


def tool_call_signature(tool_call: Any) -> ToolCallSignature:
    name = _tool_call_name(tool_call)
    arguments = _tool_call_field(tool_call, "arguments", _MISSING)
    if arguments is _MISSING or arguments is None:
        raw_arguments = _tool_call_field(tool_call, "raw_arguments", _MISSING)
        if raw_arguments is not _MISSING and raw_arguments is not None:
            arguments = raw_arguments
    return ToolCallSignature(name=name, arguments_hash=_arguments_hash(arguments))


def _tool_calls_from_turn(turn: Any) -> list[Any]:
    if isinstance(turn, Mapping):
        tool_calls = turn.get("tool_calls")
    else:
        tool_calls = getattr(turn, "tool_calls", None)

    if tool_calls is None or isinstance(tool_calls, (str, bytes, bytearray, memoryview)):
        return []

    try:
        return list(tool_calls)
    except TypeError:
        return []


def _tool_call_signatures(history: Iterable[Any]) -> list[ToolCallSignature]:
    signatures: list[ToolCallSignature] = []
    for turn in history:
        for tool_call in _tool_calls_from_turn(turn):
            try:
                signatures.append(tool_call_signature(tool_call))
            except (AttributeError, KeyError, TypeError, ValueError):
                continue
    return signatures


def detect_loop(
    history: Iterable[Any],
    window: int | None = None,
    *,
    loop_detection_window: int | None = None,
) -> bool:
    if loop_detection_window is not None:
        window = loop_detection_window
    if window is None:
        raise TypeError("window must be provided")
    if not isinstance(window, int):
        raise TypeError("window must be an integer")
    if window <= 1:
        return False

    signatures = _tool_call_signatures(history)
    if len(signatures) < window:
        return False

    recent_signatures = signatures[-window:]
    for pattern_length in (1, 2, 3):
        if pattern_length >= window:
            continue
        if window % pattern_length != 0:
            continue

        pattern = recent_signatures[:pattern_length]
        if all(
            recent_signatures[index] == pattern[index % pattern_length]
            for index in range(window)
        ):
            return True

    return False


__all__ = [
    "LOOP_DETECTION_WARNING",
    "ToolCallSignature",
    "detect_loop",
    "tool_call_signature",
]
