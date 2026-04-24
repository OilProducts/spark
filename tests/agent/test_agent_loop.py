from __future__ import annotations

import asyncio

import pytest

import agent
import unified_llm
from agent.history import history_to_messages


class _FakeCompleteClient:
    def __init__(self, responses: list[unified_llm.Response]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._responses = list(responses)

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        if not self._responses:
            raise AssertionError("unexpected complete call")
        return self._responses.pop(0)


class _PausingCompleteClient:
    def __init__(self, response: unified_llm.Response) -> None:
        self.requests: list[unified_llm.Request] = []
        self._response = response
        self.complete_started = asyncio.Event()
        self.release_complete = asyncio.Event()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        self.complete_started.set()
        await self.release_complete.wait()
        return self._response


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def test_history_to_messages_converts_turns_to_sdk_messages_without_mutating_them() -> None:
    user_text = unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text="User note")
    assistant_text = unified_llm.ContentPart(
        kind=unified_llm.ContentKind.TEXT,
        text="Assistant reply",
    )
    tool_call = unified_llm.ToolCallData(
        id="call-1",
        name="lookup",
        arguments={"value": 1},
    )
    tool_result = unified_llm.ToolResultData(
        tool_call_id="call-1",
        content={"value": 1},
        is_error=False,
        image_data=b"image-bytes",
        image_media_type="image/png",
    )
    usage = unified_llm.Usage(input_tokens=3, output_tokens=5, total_tokens=8)

    system_turn = agent.SystemTurn(content="System note")
    steering_turn = agent.SteeringTurn(content="Steer the model")
    user_turn = agent.UserTurn(content=[user_text])
    assistant_turn = agent.AssistantTurn(
        content=[assistant_text],
        tool_calls=[tool_call],
        reasoning="Assistant thinking",
        usage=usage,
        response_id="resp-1",
        finish_reason=unified_llm.FinishReason.STOP,
    )
    tool_results_turn = agent.ToolResultsTurn(result_list=[tool_result])

    messages = history_to_messages(
        [system_turn, steering_turn, user_turn, assistant_turn, tool_results_turn]
    )

    assert messages == [
        unified_llm.Message.system("System note"),
        unified_llm.Message.user("Steer the model"),
        unified_llm.Message.user([user_text]),
        unified_llm.Message.assistant(
            [
                assistant_text,
                unified_llm.ContentPart(
                    kind=unified_llm.ContentKind.THINKING,
                    thinking=unified_llm.ThinkingData(text="Assistant thinking"),
                    text="Assistant thinking",
                ),
                unified_llm.ContentPart(
                    kind=unified_llm.ContentKind.TOOL_CALL,
                    tool_call=tool_call,
                ),
            ]
        ),
        unified_llm.Message.tool_result(
            tool_call_id="call-1",
            content={"value": 1},
            is_error=False,
            image_data=b"image-bytes",
            image_media_type="image/png",
        ),
    ]

    assert system_turn.content == "System note"
    assert steering_turn.content == "Steer the model"
    assert user_turn.content == [user_text]
    assert assistant_turn.content == [assistant_text]
    assert assistant_turn.tool_calls == [tool_call]
    assert assistant_turn.reasoning == "Assistant thinking"
    assert assistant_turn.usage == usage
    assert assistant_turn.response_id == "resp-1"
    assert assistant_turn.finish_reason.reason == "stop"
    assert tool_results_turn.result_list == [tool_result]
    assert tool_results_turn.results == [tool_result]


def test_session_build_request_prepends_system_prompt_and_preserves_sdk_history() -> None:
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        provider_options_map={"temperature": 0.2},
    )
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )
    profile.tool_registry.register(tool_definition, executor=lambda arguments, env: "ok")
    session = agent.Session(
        profile=profile,
        config=agent.SessionConfig(reasoning_effort="high"),
    )

    tool_call = unified_llm.ToolCallData(
        id="call-1",
        name="lookup",
        arguments={"value": 1},
    )
    tool_result = unified_llm.ToolResultData(
        tool_call_id="call-1",
        content="done",
        is_error=False,
    )
    assistant_turn = agent.AssistantTurn(
        content="Assistant reply",
        tool_calls=[tool_call],
        reasoning="Assistant thinking",
        response_id="resp-1",
        finish_reason=unified_llm.FinishReason.STOP,
    )
    session.history = [
        agent.SystemTurn(content="History system"),
        agent.SteeringTurn(content="History steering"),
        agent.UserTurn(content="History user"),
        assistant_turn,
        agent.ToolResultsTurn(result_list=[tool_result]),
    ]

    request = session.build_request("Session system prompt")

    assert request.model == "fake-model"
    assert request.provider == "fake-provider"
    assert request.messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history),
    ]
    assert request.reasoning_effort == "high"
    assert request.provider_options == {"fake-provider": {"temperature": 0.2}}
    assert request.tools is not None
    assert len(request.tools) == 1
    assert isinstance(request.tools[0], unified_llm.Tool)
    assert request.tools[0].name == "lookup"
    assert request.tools[0].description == "Lookup values"
    assert request.tools[0].parameters == {"type": "object"}
    assert request.tool_choice is not None
    assert request.tool_choice.mode == "auto"
    assert request.tool_choice.tool_name is None
    assert session.history[3] is assistant_turn
    assert assistant_turn.response_id == "resp-1"
    assert assistant_turn.finish_reason.reason == "stop"


@pytest.mark.asyncio
async def test_session_process_input_runs_the_full_non_streaming_loop_and_reuses_history(
) -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant(
                    [
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TEXT,
                            text="Assistant reply",
                        ),
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.THINKING,
                            thinking=unified_llm.ThinkingData(text="Assistant thinking"),
                            text="Assistant thinking",
                        ),
                    ]
                ),
                finish_reason=unified_llm.FinishReason.STOP,
                usage=unified_llm.Usage(input_tokens=3, output_tokens=5, total_tokens=8),
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant(
                    [
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TEXT,
                            text="Second reply",
                        ),
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.THINKING,
                            thinking=unified_llm.ThinkingData(text="Second thinking"),
                            text="Second thinking",
                        ),
                    ]
                ),
                finish_reason=unified_llm.FinishReason.STOP,
                usage=unified_llm.Usage(input_tokens=4, output_tokens=6, total_tokens=10),
            ),
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    session.mark_awaiting_input("What is the next step?")
    assert session.state == agent.SessionState.AWAITING_INPUT
    assert session.pending_question == "What is the next step?"

    await session.process_input("Answer one")
    first_user_input = await _next_event(stream)
    assert first_user_input.kind == agent.EventKind.USER_INPUT
    assert first_user_input.data == {
        "content": "Answer one",
        "answer_to": "What is the next step?",
    }
    first_assistant_start = await _next_event(stream)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert first_assistant_start.data == {"response_id": "resp-1"}
    first_assistant_delta = await _next_event(stream)
    assert first_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_assistant_delta.data == {
        "response_id": "resp-1",
        "delta": "Assistant reply",
    }
    first_assistant_end = await _next_event(stream)
    assert first_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert first_assistant_end.data == {
        "text": "Assistant reply",
        "reasoning": "Assistant thinking",
    }
    first_processing_end = await _next_event(stream)
    assert first_processing_end.kind == agent.EventKind.PROCESSING_END
    assert first_processing_end.data == {"state": "idle"}
    assert session.pending_question is None
    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == ["Answer one", "Assistant reply"]
    assert session.history[1].response_id == "resp-1"
    assert session.history[1].finish_reason.reason == "stop"
    assert session.history[1].usage == unified_llm.Usage(
        input_tokens=3,
        output_tokens=5,
        total_tokens=8,
    )

    session.mark_awaiting_input("And then?")
    assert session.state == agent.SessionState.AWAITING_INPUT
    assert session.pending_question == "And then?"

    await session.submit("Answer two")
    second_user_input = await _next_event(stream)
    assert second_user_input.kind == agent.EventKind.USER_INPUT
    assert second_user_input.data == {
        "content": "Answer two",
        "answer_to": "And then?",
    }
    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert second_assistant_start.data == {"response_id": "resp-2"}
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "Second reply",
    }
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {
        "text": "Second reply",
        "reasoning": "Second thinking",
    }
    second_processing_end = await _next_event(stream)
    assert second_processing_end.kind == agent.EventKind.PROCESSING_END
    assert second_processing_end.data == {"state": "idle"}
    assert session.pending_question is None
    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == [
        "Answer one",
        "Assistant reply",
        "Answer two",
        "Second reply",
    ]
    assert session.history[3].response_id == "resp-2"
    assert session.history[3].finish_reason.reason == "stop"
    assert session.history[3].usage == unified_llm.Usage(
        input_tokens=4,
        output_tokens=6,
        total_tokens=10,
    )
    assert len(client.requests) == 2
    assert client.requests[0].provider == "fake-provider"
    assert client.requests[1].provider == "fake-provider"
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:1]),
    ]
    assert client.requests[1].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:3]),
    ]


@pytest.mark.asyncio
async def test_session_process_input_drains_steering_and_executes_tool_rounds() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant(
                    [
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TEXT,
                            text="Need tool",
                        ),
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.THINKING,
                            thinking=unified_llm.ThinkingData(text="Thinking about a tool"),
                            text="Thinking about a tool",
                        ),
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TOOL_CALL,
                            tool_call=unified_llm.ToolCallData(
                                id="call-1",
                                name="lookup",
                                arguments={"value": 1},
                            ),
                        ),
                    ]
                ),
                finish_reason=unified_llm.FinishReason(
                    reason=unified_llm.FinishReason.TOOL_CALLS,
                ),
                usage=unified_llm.Usage(input_tokens=6, output_tokens=4, total_tokens=10),
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("All done"),
                finish_reason=unified_llm.FinishReason.STOP,
                usage=unified_llm.Usage(input_tokens=8, output_tokens=2, total_tokens=10),
            ),
        ]
    )
    execution_environment = agent.LocalExecutionEnvironment(working_dir=".")
    tool_registry = agent.ToolRegistry()
    profile = _PromptProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=tool_registry,
        provider_options_map={"temperature": 0.2},
    )
    session = agent.Session(
        profile=profile,
        execution_env=execution_environment,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort="high"),
    )
    stream = session.events()
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup value",
        parameters={"type": "object"},
    )

    def executor(arguments: dict[str, object], environment: object) -> str:
        assert arguments == {"value": 1}
        assert environment is execution_environment
        session.steer("tool steering")
        return "tool result"

    tool_registry.register(tool_definition, executor=executor)
    session.steer("preloaded steering")

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    assert client.requests[0].provider == "fake-provider"
    assert client.requests[0].reasoning_effort == "high"
    assert client.requests[0].provider_options == {"fake-provider": {"temperature": 0.2}}
    assert client.requests[0].tool_choice is not None
    assert client.requests[0].tool_choice.mode == "auto"
    assert client.requests[0].tool_choice.tool_name is None
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Question"),
        unified_llm.Message.user("preloaded steering"),
    ]

    assert client.requests[1].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:5]),
    ]
    assert client.requests[1].reasoning_effort == "high"
    assert client.requests[1].provider_options == {"fake-provider": {"temperature": 0.2}}

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    first_steering_event = await _next_event(stream)
    assert first_steering_event.kind == agent.EventKind.STEERING_INJECTED
    assert first_steering_event.data == {"content": "preloaded steering"}
    first_assistant_start = await _next_event(stream)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert first_assistant_start.data == {"response_id": "resp-1"}
    first_assistant_delta = await _next_event(stream)
    assert first_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_assistant_delta.data == {"response_id": "resp-1", "delta": "Need tool"}
    first_assistant_end = await _next_event(stream)
    assert first_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert first_assistant_end.data == {
        "text": "Need tool",
        "reasoning": "Thinking about a tool",
    }
    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert tool_start_event.data == {"tool_call_id": "call-1", "tool_name": "lookup"}
    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-1",
        "tool_name": "lookup",
        "output": "tool result",
    }
    second_steering_event = await _next_event(stream)
    assert second_steering_event.kind == agent.EventKind.STEERING_INJECTED
    assert second_steering_event.data == {"content": "tool steering"}
    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert second_assistant_start.data == {"response_id": "resp-2"}
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {"response_id": "resp-2", "delta": "All done"}
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {"text": "All done", "reasoning": None}
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "SteeringTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "SteeringTurn",
        "AssistantTurn",
    ]
    assert session.history[2].response_id == "resp-1"
    assert session.history[2].finish_reason.reason == "tool_calls"
    assert session.history[3].result_list[0].content == "tool result"
    assert session.history[4].content == "tool steering"
    assert session.history[5].response_id == "resp-2"
    assert session.history[5].finish_reason.reason == "stop"


@pytest.mark.asyncio
async def test_session_process_input_emits_turn_limit_after_tool_round_limit() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant(
                    [
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TEXT,
                            text="Need tool",
                        ),
                        unified_llm.ContentPart(
                            kind=unified_llm.ContentKind.TOOL_CALL,
                            tool_call=unified_llm.ToolCallData(
                                id="call-1",
                                name="lookup",
                                arguments={"value": 1},
                            ),
                        ),
                    ]
                ),
                finish_reason=unified_llm.FinishReason(
                    reason=unified_llm.FinishReason.TOOL_CALLS,
                ),
            )
        ]
    )
    tool_registry = agent.ToolRegistry()
    profile = _PromptProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=tool_registry,
    )
    tool_registry.register(
        agent.ToolDefinition(
            name="lookup",
            description="Lookup value",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, environment: "tool result",
    )
    session = agent.Session(
        profile=profile,
        llm_client=client,
        config=agent.SessionConfig(max_tool_rounds_per_input=1),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    assert len(client.requests) == 1

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    turn_limit_event = await _next_event(stream)
    assert turn_limit_event.kind == agent.EventKind.TURN_LIMIT
    assert turn_limit_event.data == {
        "state": "idle",
        "round_count": 1,
        "total_turns": 3,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
    ]
    assert session.history[2].result_list[0].content == "tool result"


@pytest.mark.asyncio
async def test_session_process_input_emits_turn_limit_when_max_turns_reached(
    ) -> None:
    client = _FakeCompleteClient([])
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
        config=agent.SessionConfig(max_turns=1),
        history=[agent.UserTurn(content="previous")],
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("new input")

    assert client.requests == []

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "new input"}
    turn_limit_event = await _next_event(stream)
    assert turn_limit_event.kind == agent.EventKind.TURN_LIMIT
    assert turn_limit_event.data == {
        "state": "idle",
        "round_count": 0,
        "total_turns": 2,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == ["previous", "new input"]


@pytest.mark.asyncio
async def test_session_process_input_abort_cancels_pending_model_call() -> None:
    client = _PausingCompleteClient(
        unified_llm.Response(
            id="resp-1",
            model="fake-model",
            provider="fake-provider",
            message=unified_llm.Message.assistant("done"),
            finish_reason=unified_llm.FinishReason.STOP,
        )
    )
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    processing_task = asyncio.create_task(session.process_input("Question"))

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}

    await asyncio.wait_for(client.complete_started.wait(), timeout=1)
    assert len(client.requests) == 1

    await session.abort()
    client.release_complete.set()

    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    with pytest.raises(agent.SessionAbortedError):
        await processing_task

    assert session.state == agent.SessionState.CLOSED
    assert session.abort_signaled is True
    assert [type(turn).__name__ for turn in session.history] == ["UserTurn"]
