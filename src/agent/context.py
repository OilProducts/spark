from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any, Protocol

from .events import EventKind, SessionEvent


class _ContextSession(Protocol):
    history: Iterable[Any]
    provider_profile: Any

    def emit_event(self, event: SessionEvent) -> None: ...


def _character_count(value: Any) -> int:
    if value is None:
        return 0
    if isinstance(value, str):
        return len(value)
    if isinstance(value, bytes):
        return len(value)

    text = getattr(value, "text", None)
    if isinstance(text, str):
        return len(text)

    result_list = getattr(value, "result_list", None)
    if result_list is not None:
        return sum(_character_count(item) for item in result_list)

    results = getattr(value, "results", None)
    if results is not None:
        return sum(_character_count(item) for item in results)

    content = getattr(value, "content", None)
    if content is not None:
        return _character_count(content)

    if isinstance(value, Mapping):
        return sum(_character_count(item) for item in value.values())

    if isinstance(value, Iterable):
        return sum(
            _character_count(item)
            for item in value
            if not isinstance(item, (str, bytes, bytearray, memoryview))
        )

    return len(str(value))


def _estimate_context_tokens(history: Iterable[Any]) -> float:
    total_chars = sum(_character_count(turn) for turn in history)
    return total_chars / 4


def check_context_usage(session: _ContextSession) -> bool:
    provider_profile = getattr(session, "provider_profile", None)
    context_window_size = getattr(provider_profile, "context_window_size", None)
    if not context_window_size or context_window_size <= 0:
        return False

    if getattr(session, "_context_warning_emitted", False):
        return False

    approx_tokens = _estimate_context_tokens(getattr(session, "history", []))
    threshold = context_window_size * 0.8
    if approx_tokens <= threshold:
        return False

    percent = round(approx_tokens / context_window_size * 100)
    event = SessionEvent(
        kind=EventKind.WARNING,
        session_id=getattr(session, "id", getattr(session, "session_id", None)),
        data={"message": f"Context usage at ~{percent}% of context window"},
    )
    try:
        setattr(session, "_context_warning_emitted", True)
    except (AttributeError, TypeError):
        pass
    session.emit_event(event)
    return True


__all__ = ["check_context_usage"]
