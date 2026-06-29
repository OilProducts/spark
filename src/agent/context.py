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


def _session_prompt_text(session: Any) -> str:
    for attribute in ("system_prompt_snapshot", "_system_prompt", "system_prompt"):
        value = getattr(session, attribute, None)
        if isinstance(value, str):
            return value
    return ""


def _estimate_context_usage(
    history: Iterable[Any],
    *,
    prompt: str = "",
    context_window_size: int,
) -> dict[str, Any]:
    approximate_characters = _character_count(prompt) + sum(
        _character_count(turn) for turn in history
    )
    approximate_tokens = approximate_characters / 4
    threshold_tokens = context_window_size * 0.8
    usage_ratio = approximate_tokens / context_window_size
    return {
        "approximate_characters": approximate_characters,
        "approximate_tokens": approximate_tokens,
        "threshold_tokens": threshold_tokens,
        "threshold_ratio": 0.8,
        "context_window_size": context_window_size,
        "usage_ratio": usage_ratio,
    }


def check_context_usage(session: _ContextSession) -> bool:
    provider_profile = getattr(session, "provider_profile", None)
    context_window_size = getattr(provider_profile, "context_window_size", None)
    if not context_window_size or context_window_size <= 0:
        return False

    usage = _estimate_context_usage(
        getattr(session, "history", []),
        prompt=_session_prompt_text(session),
        context_window_size=context_window_size,
    )
    if usage["approximate_tokens"] <= usage["threshold_tokens"]:
        return False

    percent = int(usage["usage_ratio"] * 100 + 0.5)
    event = SessionEvent(
        kind=EventKind.WARNING,
        session_id=getattr(session, "id", getattr(session, "session_id", None)),
        data={
            "message": f"Context usage at ~{percent}% of context window",
            "usage": usage,
        },
    )
    session.emit_event(event)
    return True


__all__ = ["check_context_usage"]
