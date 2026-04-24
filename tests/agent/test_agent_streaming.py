from __future__ import annotations

import asyncio

import pytest

import agent
import unified_llm


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def _tool_call(
    call_id: str,
    name: str,
    arguments: dict[str, object],
) -> unified_llm.ToolCall:
    return unified_llm.ToolCall(
        id=call_id,
        name=name,
        arguments=arguments,
        type="function",
    )


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


class _FakeStreamingClient:
    def __init__(self, stream_event_groups: list[list[unified_llm.StreamEvent]]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._stream_event_groups = [list(group) for group in stream_event_groups]

    def stream(self, request: unified_llm.Request):
        self.requests.append(request)
        if not self._stream_event_groups:
            raise AssertionError("unexpected stream call")

        events = self._stream_event_groups.pop(0)

        async def _events():
            for event in events:
                yield event

        return _events()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        raise AssertionError("streaming sessions must not call complete()")


class _PausingStreamingClient:
    def __init__(self, stream_events: list[unified_llm.StreamEvent]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._stream_events = list(stream_events)
        self.resume_stream = asyncio.Event()

    def stream(self, request: unified_llm.Request):
        self.requests.append(request)
        events = list(self._stream_events)

        async def _events():
            for event in events:
                yield event
                if (
                    event.type == unified_llm.StreamEventType.TEXT_DELTA
                    and event.delta == "world"
                ):
                    await self.resume_stream.wait()

        return _events()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        raise AssertionError("streaming sessions must not call complete()")


@pytest.mark.asyncio
async def test_session_process_input_streaming_emits_text_events_while_stream_is_in_flight(
) -> None:
    finish_warning = unified_llm.Warning(
        message="soft limit approached",
        code="rate_limit",
    )
    client = _PausingStreamingClient(
        [
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.STREAM_START,
                response=unified_llm.Response(
                    id="resp-1",
                    model="fake-model",
                    provider="fake-provider",
                ),
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_START,
                delta="Hello ",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_DELTA,
                delta="world",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_END,
                delta="!",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.FINISH,
                finish_reason=unified_llm.FinishReason.STOP,
                usage=unified_llm.Usage(
                    input_tokens=2,
                    output_tokens=4,
                    total_tokens=6,
                ),
                response=unified_llm.Response(
                    id="resp-1",
                    model="fake-model",
                    provider="fake-provider",
                    raw={"finish": True},
                    warnings=[finish_warning],
                ),
            ),
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(
            id="fake-provider",
            model="fake-model",
            supports_streaming=True,
        ),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    processing_task = asyncio.create_task(session.process_input("Question"))

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}

    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert assistant_start_event.data == {"response_id": "resp-1"}

    first_delta_event = await _next_event(stream)
    assert first_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_delta_event.data == {"response_id": "resp-1", "delta": "Hello "}

    assert not processing_task.done()

    second_delta_event = await _next_event(stream)
    assert second_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_delta_event.data == {"response_id": "resp-1", "delta": "world"}

    client.resume_stream.set()

    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {
        "text": "Hello world!",
        "reasoning": None,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    await processing_task

    assert len(client.requests) == 1
    assert client.requests[0].provider == "fake-provider"
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Question"),
    ]
    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
    ]
    assistant_turn = session.history[1]
    assert assistant_turn.text == "Hello world!"
    assert assistant_turn.response_id == "resp-1"
    assert assistant_turn.finish_reason.reason == "stop"
    assert assistant_turn.usage == unified_llm.Usage(
        input_tokens=2,
        output_tokens=4,
        total_tokens=6,
    )
    assert assistant_turn.warnings == [finish_warning]
    assert assistant_turn.raw == {"finish": True}


@pytest.mark.asyncio
async def test_session_process_input_streaming_abort_stops_before_natural_completion(
) -> None:
    client = _PausingStreamingClient(
        [
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.STREAM_START,
                response=unified_llm.Response(
                    id="resp-1",
                    model="fake-model",
                    provider="fake-provider",
                ),
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_START,
                delta="Hello ",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_DELTA,
                delta="world",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_END,
                delta="!",
            ),
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.FINISH,
                finish_reason=unified_llm.FinishReason.STOP,
                usage=unified_llm.Usage(
                    input_tokens=2,
                    output_tokens=4,
                    total_tokens=6,
                ),
                response=unified_llm.Response(
                    id="resp-1",
                    model="fake-model",
                    provider="fake-provider",
                    raw={"finish": True},
                ),
            ),
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(
            id="fake-provider",
            model="fake-model",
            supports_streaming=True,
        ),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    processing_task = asyncio.create_task(session.process_input("Question"))

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}

    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert assistant_start_event.data == {"response_id": "resp-1"}

    first_delta_event = await _next_event(stream)
    assert first_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_delta_event.data == {"response_id": "resp-1", "delta": "Hello "}

    second_delta_event = await _next_event(stream)
    assert second_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_delta_event.data == {"response_id": "resp-1", "delta": "world"}

    await session.abort()
    client.resume_stream.set()

    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    with pytest.raises(agent.SessionAbortedError):
        await processing_task

    assert session.state == agent.SessionState.CLOSED
    assert session.abort_signaled is True
    assert [type(turn).__name__ for turn in session.history] == ["UserTurn"]


@pytest.mark.asyncio
async def test_session_process_input_streaming_reconstructs_text_and_metadata() -> None:
    client = _FakeStreamingClient(
        [
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.STREAM_START,
                    response=unified_llm.Response(
                        id="resp-1",
                        model="fake-model",
                        provider="fake-provider",
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_START,
                    text_id="text-1",
                    delta="Hello ",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="world",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_END,
                    delta="!",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.REASONING_START,
                    reasoning_delta="think",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.REASONING_END,
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.FINISH,
                    finish_reason=unified_llm.FinishReason.STOP,
                    usage=unified_llm.Usage(
                        input_tokens=2,
                        output_tokens=4,
                        total_tokens=6,
                    ),
                    response=unified_llm.Response(
                        id="resp-1",
                        model="fake-model",
                        provider="fake-provider",
                        raw={"finish": True},
                    ),
                ),
            ]
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(
            id="fake-provider",
            model="fake-model",
            supports_streaming=True,
        ),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert assistant_start_event.data == {"response_id": "resp-1"}
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {"response_id": "resp-1", "delta": "Hello "}
    second_delta_event = await _next_event(stream)
    assert second_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_delta_event.data == {"response_id": "resp-1", "delta": "world"}
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {
        "text": "Hello world!",
        "reasoning": "think",
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert len(client.requests) == 1
    assert client.requests[0].provider == "fake-provider"
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Question"),
    ]
    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
    ]
    assistant_turn = session.history[1]
    assert assistant_turn.text == "Hello world!"
    assert assistant_turn.reasoning == "think"
    assert assistant_turn.response_id == "resp-1"
    assert assistant_turn.finish_reason.reason == "stop"
    assert assistant_turn.usage == unified_llm.Usage(
        input_tokens=2,
        output_tokens=4,
        total_tokens=6,
    )
    assert assistant_turn.raw == {"finish": True}
    assert assistant_turn.warnings == []
    assert assistant_turn.error is None


@pytest.mark.asyncio
async def test_session_process_input_streaming_tool_round_preserves_result_order(
) -> None:
    client = _FakeStreamingClient(
        [
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.STREAM_START,
                    response=unified_llm.Response(
                        id="resp-1",
                        model="fake-model",
                        provider="fake-provider",
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_START,
                    delta="Need ",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="tools",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_END,
                    delta="!",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TOOL_CALL_START,
                    tool_call=_tool_call(
                        "call-1",
                        "first_tool",
                        {"value": 1},
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TOOL_CALL_END,
                    tool_call=_tool_call(
                        "call-1",
                        "first_tool",
                        {"value": 1},
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TOOL_CALL_START,
                    tool_call=_tool_call(
                        "call-2",
                        "second_tool",
                        {"value": 2},
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TOOL_CALL_END,
                    tool_call=_tool_call(
                        "call-2",
                        "second_tool",
                        {"value": 2},
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.FINISH,
                    finish_reason=unified_llm.FinishReason(
                        reason=unified_llm.FinishReason.TOOL_CALLS,
                    ),
                    response=unified_llm.Response(
                        id="resp-1",
                        model="fake-model",
                        provider="fake-provider",
                    ),
                ),
            ],
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.STREAM_START,
                    response=unified_llm.Response(
                        id="resp-2",
                        model="fake-model",
                        provider="fake-provider",
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_START,
                    delta="All ",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="done",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_END,
                    delta=".",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.FINISH,
                    finish_reason=unified_llm.FinishReason.STOP,
                    usage=unified_llm.Usage(
                        input_tokens=6,
                        output_tokens=2,
                        total_tokens=8,
                    ),
                    response=unified_llm.Response(
                        id="resp-2",
                        model="fake-model",
                        provider="fake-provider",
                        raw={"finish": True},
                    ),
                ),
            ],
        ]
    )
    execution_environment = agent.LocalExecutionEnvironment(working_dir=".")
    tool_registry = agent.ToolRegistry()
    profile = _PromptProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=tool_registry,
        supports_streaming=True,
    )
    session = agent.Session(
        profile=profile,
        execution_env=execution_environment,
        llm_client=client,
    )
    stream = session.events()

    tool_registry.register(
        agent.ToolDefinition(
            name="first_tool",
            description="First tool",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, env: "first result",
    )
    tool_registry.register(
        agent.ToolDefinition(
            name="second_tool",
            description="Second tool",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, env: "second result",
    )

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {"response_id": "resp-1", "delta": "Need "}
    second_delta_event = await _next_event(stream)
    assert second_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_delta_event.data == {"response_id": "resp-1", "delta": "tools"}
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {"text": "Need tools!", "reasoning": None}
    first_tool_start = await _next_event(stream)
    assert first_tool_start.kind == agent.EventKind.TOOL_CALL_START
    assert first_tool_start.data == {"tool_call_id": "call-1", "tool_name": "first_tool"}
    first_tool_end = await _next_event(stream)
    assert first_tool_end.kind == agent.EventKind.TOOL_CALL_END
    assert first_tool_end.data == {
        "tool_call_id": "call-1",
        "tool_name": "first_tool",
        "output": "first result",
    }
    second_tool_start = await _next_event(stream)
    assert second_tool_start.kind == agent.EventKind.TOOL_CALL_START
    assert second_tool_start.data == {"tool_call_id": "call-2", "tool_name": "second_tool"}
    second_tool_end = await _next_event(stream)
    assert second_tool_end.kind == agent.EventKind.TOOL_CALL_END
    assert second_tool_end.data == {
        "tool_call_id": "call-2",
        "tool_name": "second_tool",
        "output": "second result",
    }
    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert second_assistant_start.data == {"response_id": "resp-2"}
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {"response_id": "resp-2", "delta": "All "}
    third_assistant_delta = await _next_event(stream)
    assert third_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert third_assistant_delta.data == {"response_id": "resp-2", "delta": "done"}
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {"text": "All done.", "reasoning": None}
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert len(client.requests) == 2
    assert client.requests[0].provider == "fake-provider"
    assert client.requests[0].tool_choice is not None
    assert client.requests[0].tool_choice.mode == "auto"
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Question"),
    ]
    second_request_messages = client.requests[1].messages
    assert [message.role for message in second_request_messages] == [
        unified_llm.Role.SYSTEM,
        unified_llm.Role.USER,
        unified_llm.Role.ASSISTANT,
        unified_llm.Role.TOOL,
        unified_llm.Role.TOOL,
    ]
    assert second_request_messages[0].text == "Session system prompt"
    assert second_request_messages[1].text == "Question"
    assert second_request_messages[2].text == "Need tools!"
    assert [
        part.tool_call.id
        for part in second_request_messages[2].content
        if part.tool_call is not None
    ] == ["call-1", "call-2"]
    assert [
        part.tool_call.name
        for part in second_request_messages[2].content
        if part.tool_call is not None
    ] == ["first_tool", "second_tool"]
    assert [message.tool_call_id for message in second_request_messages[3:]] == [
        "call-1",
        "call-2",
    ]
    assert [
        message.content[0].tool_result.content
        for message in second_request_messages[3:]
    ] == [
        "first result",
        "second result",
    ]
    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
    ]
    first_assistant_turn = session.history[1]
    assert [tool_call.name for tool_call in first_assistant_turn.tool_calls] == [
        "first_tool",
        "second_tool",
    ]
    assert first_assistant_turn.text == "Need tools!"
    assert first_assistant_turn.finish_reason.reason == "tool_calls"
    assert session.history[2].result_list[0].content == "first result"
    assert session.history[2].result_list[1].content == "second result"
    assert session.history[3].text == "All done."
    assert session.history[3].finish_reason.reason == "stop"
    assert session.history[3].raw == {"finish": True}


@pytest.mark.asyncio
async def test_session_process_input_streaming_error_closes_session_and_records_partial_turn(
) -> None:
    error = unified_llm.SDKError("boom")
    client = _FakeStreamingClient(
        [
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.STREAM_START,
                    response=unified_llm.Response(
                        id="resp-err",
                        model="fake-model",
                        provider="fake-provider",
                    ),
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="partial",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.ERROR,
                    error=error,
                    raw={"error": "boom"},
                    response=unified_llm.Response(
                        id="resp-err",
                        model="fake-model",
                        provider="fake-provider",
                        raw={"partial": True},
                    ),
                ),
            ]
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(
            id="fake-provider",
            model="fake-model",
            supports_streaming=True,
        ),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with pytest.raises(unified_llm.SDKError, match="boom"):
        await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {
        "response_id": "resp-err",
        "delta": "partial",
    }
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {"text": "partial", "reasoning": None}
    error_event = await _next_event(stream)
    assert error_event.kind == agent.EventKind.ERROR
    assert error_event.data == {"error": "boom"}
    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    assert session.state == agent.SessionState.CLOSED
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
    ]
    assistant_turn = session.history[1]
    assert assistant_turn.text == "partial"
    assert assistant_turn.response_id == "resp-err"
    assert assistant_turn.finish_reason.reason == "error"
    assert assistant_turn.raw == {"partial": True}
    assert assistant_turn.error is error
