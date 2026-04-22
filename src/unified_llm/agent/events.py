from __future__ import annotations

import asyncio
import logging
from collections.abc import AsyncIterator, Mapping
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any
from uuid import UUID

logger = logging.getLogger(__name__)


def _utcnow() -> datetime:
    return datetime.now(UTC)


def _coerce_event_kind(kind: EventKind | str) -> EventKind | str:
    if isinstance(kind, EventKind):
        return kind
    if isinstance(kind, str):
        try:
            return EventKind(kind)
        except ValueError:
            try:
                return EventKind[kind]
            except KeyError:
                return kind
    raise TypeError("kind must be an EventKind or string")


def _coerce_uuid(value: UUID | str | None) -> UUID | str | None:
    if not isinstance(value, str):
        return value
    try:
        return UUID(value)
    except ValueError:
        return value


class EventKind(StrEnum):
    SESSION_START = "session_start"
    SESSION_END = "session_end"
    USER_INPUT = "user_input"
    PROCESSING_END = "processing_end"
    ASSISTANT_TEXT_START = "assistant_text_start"
    ASSISTANT_TEXT_DELTA = "assistant_text_delta"
    ASSISTANT_TEXT_END = "assistant_text_end"
    TOOL_CALL_START = "tool_call_start"
    TOOL_CALL_OUTPUT_DELTA = "tool_call_output_delta"
    TOOL_CALL_END = "tool_call_end"
    STEERING_INJECTED = "steering_injected"
    TURN_LIMIT = "turn_limit"
    LOOP_DETECTION = "loop_detection"
    WARNING = "warning"
    ERROR = "error"


@dataclass
class SessionEvent:
    kind: EventKind | str
    timestamp: datetime = field(default_factory=_utcnow)
    session_id: UUID | str | None = None
    data: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.kind = _coerce_event_kind(self.kind)
        self.session_id = _coerce_uuid(self.session_id)
        self.data = dict(self.data)


class _SessionEventStream(AsyncIterator[SessionEvent]):
    def __init__(self, queue: asyncio.Queue[SessionEvent]) -> None:
        self._queue = queue

    def __aiter__(self) -> _SessionEventStream:
        return self

    async def __anext__(self) -> SessionEvent:
        return await self._queue.get()


__all__ = ["EventKind", "SessionEvent"]
