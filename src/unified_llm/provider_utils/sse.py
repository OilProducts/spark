from __future__ import annotations

import logging
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass, field
from typing import Any

logger = logging.getLogger(__name__)


@dataclass(slots=True)
class SSEEvent:
    type: str
    data: str | None = None
    id: str | None = None
    retry: int | None = None
    comment: str | None = None
    raw: str = ""
    data_lines: tuple[str, ...] = ()

    @property
    def event(self) -> str:
        return self.type


def _decode_chunk(chunk: bytes | bytearray, *, field_name: str) -> str:
    try:
        return bytes(chunk).decode("utf-8")
    except UnicodeDecodeError:
        logger.debug("Unable to decode %s as UTF-8", field_name, exc_info=True)
        return bytes(chunk).decode("utf-8", errors="replace")


def _normalize_line_source(source: Any) -> Iterator[str]:
    if source is None:
        raise TypeError("source must be a string, bytes, or iterable of strings/bytes")

    if isinstance(source, (bytes, bytearray)):
        yield from _decode_chunk(source, field_name="SSE payload").splitlines()
        return

    if isinstance(source, str):
        yield from source.splitlines()
        return

    try:
        iterator = iter(source)
    except TypeError as error:
        raise TypeError(
            "source must be a string, bytes, or iterable of strings/bytes"
        ) from error

    for chunk in iterator:
        if chunk is None:
            continue
        if isinstance(chunk, (bytes, bytearray)):
            chunk = _decode_chunk(chunk, field_name="SSE chunk")
        elif not isinstance(chunk, str):
            logger.debug("Unexpected SSE chunk type: %s", type(chunk).__name__)
            raise TypeError("SSE chunks must be strings or bytes")

        if chunk == "":
            yield ""
            continue

        yield from chunk.splitlines()


def _strip_sse_value(value: str) -> str:
    return value[1:] if value.startswith(" ") else value


def _parse_retry(value: str) -> int | None:
    try:
        return int(value)
    except ValueError:
        logger.debug("Unable to parse SSE retry value %r", value, exc_info=True)
        return None


@dataclass(slots=True)
class _SSEState:
    event_type: str | None = None
    event_seen: bool = False
    data_lines: list[str] = field(default_factory=list)
    comments: list[str] = field(default_factory=list)
    event_id: str | None = None
    id_seen: bool = False
    retry: int | None = None
    retry_seen: bool = False
    raw_lines: list[str] = field(default_factory=list)

    def _emit_event(self) -> SSEEvent | None:
        if not (
            self.event_seen
            or self.data_lines
            or self.comments
            or self.id_seen
            or self.retry_seen
        ):
            return None

        return SSEEvent(
            type=self.event_type if self.event_seen else "message",
            data="\n".join(self.data_lines) if self.data_lines else None,
            id=self.event_id,
            retry=self.retry if self.retry_seen else None,
            comment="\n".join(self.comments) if self.comments else None,
            raw="\n".join(self.raw_lines),
            data_lines=tuple(self.data_lines),
        )

    def _reset(self) -> None:
        self.event_type = None
        self.event_seen = False
        self.data_lines = []
        self.comments = []
        self.event_id = None
        self.id_seen = False
        self.retry = None
        self.retry_seen = False
        self.raw_lines = []

    def feed_line(self, line: str) -> SSEEvent | None:
        if line.endswith("\r"):
            line = line[:-1]

        if line == "":
            event = self._emit_event()
            if event is not None:
                self._reset()
                return event
            self._reset()
            return None

        self.raw_lines.append(line)

        if line.startswith(":"):
            self.comments.append(_strip_sse_value(line[1:]))
            return None

        field_name, separator, value = line.partition(":")
        if separator:
            value = _strip_sse_value(value)
        else:
            value = ""
        field_name = field_name.strip()

        if field_name == "event":
            self.event_type = value
            self.event_seen = True
        elif field_name == "data":
            self.data_lines.append(value)
        elif field_name == "id":
            self.event_id = value
            self.id_seen = True
        elif field_name == "retry":
            parsed_retry = _parse_retry(value)
            if parsed_retry is not None:
                self.retry = parsed_retry
                self.retry_seen = True

        return None

    def finish(self) -> SSEEvent | None:
        event = self._emit_event()
        if event is not None:
            self._reset()
        return event


def _iter_sse_event_lines(source: Any) -> Iterator[str]:
    yield from _normalize_line_source(source)


async def _aiter_sse_event_lines(source: Any) -> AsyncIterator[str]:
    if source is None:
        raise TypeError("source must be a string, bytes, or iterable of strings/bytes")

    if isinstance(source, (bytes, bytearray)):
        for line in _decode_chunk(source, field_name="SSE payload").splitlines():
            yield line
        return

    if isinstance(source, str):
        for line in source.splitlines():
            yield line
        return

    try:
        iterator = aiter(source)
    except TypeError:
        for line in _normalize_line_source(source):
            yield line
        return

    async for chunk in iterator:
        if chunk is None:
            continue
        if isinstance(chunk, (bytes, bytearray)):
            chunk = _decode_chunk(chunk, field_name="SSE chunk")
        elif not isinstance(chunk, str):
            logger.debug("Unexpected SSE chunk type: %s", type(chunk).__name__)
            raise TypeError("SSE chunks must be strings or bytes")

        if chunk == "":
            yield ""
            continue

        for line in chunk.splitlines():
            yield line


def iter_sse_events(source: Any) -> Iterator[SSEEvent]:
    state = _SSEState()
    for line in _iter_sse_event_lines(source):
        event = state.feed_line(line)
        if event is not None:
            yield event

    event = state.finish()
    if event is not None:
        yield event


async def aiter_sse_events(source: Any) -> AsyncIterator[SSEEvent]:
    state = _SSEState()
    async for line in _aiter_sse_event_lines(source):
        event = state.feed_line(line)
        if event is not None:
            yield event

    event = state.finish()
    if event is not None:
        yield event


def parse_sse_events(source: Any) -> Iterator[SSEEvent]:
    return iter_sse_events(source)


parse_sse = parse_sse_events


__all__ = [
    "SSEEvent",
    "aiter_sse_events",
    "iter_sse_events",
    "parse_sse",
    "parse_sse_events",
]
