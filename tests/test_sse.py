from __future__ import annotations

from collections.abc import AsyncIterator

import pytest

import unified_llm.provider_utils as provider_utils


def test_parse_sse_events_preserves_boundaries_retry_comments_and_raw_payloads() -> None:
    payload = (
        ": keep-alive\n"
        "event: response.output_text.delta\n"
        "data: {\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}\n"
        "data: {\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}\n"
        "retry: 1500\n"
        "\n"
        "data: plain text\n"
        "\n"
    )

    events = list(provider_utils.parse_sse_events(payload))

    assert len(events) == 2
    first, second = events
    assert first.type == "response.output_text.delta"
    assert first.event == "response.output_text.delta"
    assert first.comment == "keep-alive"
    assert first.retry == 1500
    assert first.data == (
        "{\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}\n"
        "{\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}"
    )
    assert first.data_lines == (
        "{\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}",
        "{\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}",
    )
    assert "response.output_text.delta" in first.raw

    assert second.type == "message"
    assert second.data == "plain text"
    assert second.comment is None
    assert second.retry is None
    assert second.raw == "data: plain text"


def test_parse_sse_events_ignores_invalid_retry_values_and_keeps_late_event_type() -> None:
    payload = (
        "retry: invalid\n"
        "data: one\n"
        "event: custom.event\n"
        "\n"
    )

    events = list(provider_utils.parse_sse(payload))

    assert len(events) == 1
    event = events[0]
    assert event.type == "custom.event"
    assert event.data == "one"
    assert event.retry is None
    assert event.raw == "retry: invalid\ndata: one\nevent: custom.event"


@pytest.mark.asyncio
async def test_aiter_sse_events_preserves_boundaries_retry_comments_and_raw_payloads() -> None:
    async def source() -> AsyncIterator[str]:
        yield ": keep-alive\n"
        yield "event: response.output_text.delta\n"
        yield 'data: {"type": "response.output_text.delta", "delta": "Hel"}\n'
        yield 'data: {"type": "response.output_text.delta", "delta": "lo"}\n'
        yield "retry: 1500\n"
        yield "\n"
        yield "data: plain text\n"
        yield "\n"

    events = [event async for event in provider_utils.aiter_sse_events(source())]

    assert len(events) == 2
    first, second = events
    assert first.type == "response.output_text.delta"
    assert first.event == "response.output_text.delta"
    assert first.comment == "keep-alive"
    assert first.retry == 1500
    assert first.data == (
        '{"type": "response.output_text.delta", "delta": "Hel"}\n'
        '{"type": "response.output_text.delta", "delta": "lo"}'
    )
    assert first.data_lines == (
        '{"type": "response.output_text.delta", "delta": "Hel"}',
        '{"type": "response.output_text.delta", "delta": "lo"}',
    )
    assert "response.output_text.delta" in first.raw

    assert second.type == "message"
    assert second.data == "plain text"
    assert second.comment is None
    assert second.retry is None
    assert second.raw == "data: plain text"
