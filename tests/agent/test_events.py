from __future__ import annotations

import asyncio
from uuid import uuid4

import pytest

import agent


def test_agent_event_exports_are_available_and_distinct_from_sdk_stream_events() -> None:
    expected_names = {
        "SESSION_START",
        "SESSION_END",
        "USER_INPUT",
        "PROCESSING_END",
        "ASSISTANT_TEXT_START",
        "ASSISTANT_TEXT_DELTA",
        "ASSISTANT_TEXT_END",
        "TOOL_CALL_START",
        "TOOL_CALL_OUTPUT_DELTA",
        "TOOL_CALL_END",
        "STEERING_INJECTED",
        "TURN_LIMIT",
        "LOOP_DETECTION",
        "WARNING",
        "ERROR",
    }

    assert expected_names == {member.name for member in agent.EventKind}
    assert hasattr(agent, "SessionEvent")
    assert agent.SessionEvent is not agent.EventKind
    assert "EventKind" in agent.__all__
    assert "SessionEvent" in agent.__all__


def test_session_event_preserves_payload_shape_and_custom_data() -> None:
    event = agent.SessionEvent(
        kind=agent.EventKind.SESSION_START,
        session_id=uuid4(),
        data={"step": "start", "count": 1},
    )

    assert event.kind == agent.EventKind.SESSION_START
    assert event.session_id is not None
    assert event.data == {"step": "start", "count": 1}
    assert event.timestamp.tzinfo is not None


@pytest.mark.asyncio
async def test_session_events_reads_from_the_public_queue() -> None:
    session = agent.Session()
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    event = agent.SessionEvent(
        kind=agent.EventKind.USER_INPUT,
        session_id=session.id,
        data={"content": "hello"},
    )

    session.emit_event(event)

    assert hasattr(stream, "__aiter__")
    assert await asyncio.wait_for(anext(stream), timeout=1) == event


@pytest.mark.asyncio
async def test_session_close_brackets_the_public_event_stream() -> None:
    session = agent.Session()
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.close()

    end_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}
    assert session.state == agent.SessionState.CLOSED


@pytest.mark.asyncio
async def test_session_event_iteration_preserves_fifo_order() -> None:
    session = agent.Session()
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    first = agent.SessionEvent(
        kind=agent.EventKind.USER_INPUT,
        session_id=session.id,
        data={"step": "first"},
    )
    second = agent.SessionEvent(
        kind=agent.EventKind.PROCESSING_END,
        session_id=session.id,
        data={"step": "second"},
    )

    session.emit_event(first)
    session.emit_event(second)

    observed = []
    async for event in stream:
        observed.append(event)
        if len(observed) == 2:
            break

    assert observed == [first, second]
