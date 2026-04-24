from __future__ import annotations

import asyncio

import pytest

import agent
import unified_llm
from agent.loop_detection import (
    LOOP_DETECTION_WARNING,
    ToolCallSignature,
    detect_loop,
    tool_call_signature,
)


def _tool_call(
    *,
    call_id: str,
    name: str,
    arguments: dict[str, object] | str,
) -> unified_llm.ToolCallData:
    return unified_llm.ToolCallData(id=call_id, name=name, arguments=arguments)


def _tool_result(tool_call: unified_llm.ToolCallData) -> unified_llm.ToolResultData:
    return unified_llm.ToolResultData(
        tool_call_id=tool_call.id,
        content="tool result",
        is_error=False,
    )


def _history_from_tool_calls(
    tool_calls: list[unified_llm.ToolCallData],
    *,
    prefix: list[unified_llm.ToolCallData] | None = None,
) -> list[object]:
    history: list[object] = []
    for call in prefix or []:
        history.append(agent.AssistantTurn(content="prefix", tool_calls=[call]))
        history.append(agent.ToolResultsTurn(result_list=[_tool_result(call)]))
    for call in tool_calls:
        history.append(agent.AssistantTurn(content="tool", tool_calls=[call]))
        history.append(agent.ToolResultsTurn(result_list=[_tool_result(call)]))
    return history


class _FakeCompleteClient:
    def __init__(self, responses: list[unified_llm.Response]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._responses = list(responses)

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        if not self._responses:
            raise AssertionError("unexpected complete call")
        return self._responses.pop(0)


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def _tool_call_response(
    *,
    response_id: str,
    text: str,
    tool_call: unified_llm.ToolCallData,
) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="fake-model",
        provider="fake-provider",
        message=unified_llm.Message.assistant(
            [
                unified_llm.ContentPart(
                    kind=unified_llm.ContentKind.TEXT,
                    text=text,
                ),
                unified_llm.ContentPart(
                    kind=unified_llm.ContentKind.TOOL_CALL,
                    tool_call=tool_call,
                ),
            ]
        ),
        finish_reason=unified_llm.FinishReason(
            reason=unified_llm.FinishReason.TOOL_CALLS,
        ),
    )


def _final_response(*, response_id: str, text: str) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="fake-model",
        provider="fake-provider",
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason(reason=unified_llm.FinishReason.STOP),
    )


def test_tool_call_signature_is_stable_across_mapping_order_and_sdk_shapes() -> None:
    first = _tool_call(
        call_id="call-1",
        name="lookup",
        arguments={"b": 2, "a": {"y": 2, "x": 1}},
    )
    second = unified_llm.ToolCall(
        id="call-2",
        name="lookup",
        arguments='{"a": {"x": 1, "y": 2}, "b": 2}',
    )

    first_signature = tool_call_signature(first)
    second_signature = tool_call_signature(second)

    assert isinstance(first_signature, ToolCallSignature)
    assert first_signature == second_signature
    assert first_signature.name == "lookup"
    assert first_signature.arguments_hash == second_signature.arguments_hash


@pytest.mark.parametrize(
    ("prefix_call", "repeating_calls", "window"),
    [
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="lookup", arguments={"value": 1}),
            ],
            2,
        ),
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-3", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-4", name="summarize", arguments={"value": 2}),
            ],
            4,
        ),
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-3", name="expand", arguments={"value": 3}),
                _tool_call(call_id="call-4", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-5", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-6", name="expand", arguments={"value": 3}),
            ],
            6,
        ),
    ],
)
def test_detect_loop_matches_repeating_patterns(
    prefix_call: unified_llm.ToolCallData,
    repeating_calls: list[unified_llm.ToolCallData],
    window: int,
) -> None:
    history = _history_from_tool_calls(repeating_calls, prefix=[prefix_call])

    assert detect_loop(history, loop_detection_window=window) is True


@pytest.mark.parametrize(
    ("prefix_call", "near_miss_calls", "window"),
    [
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="lookup", arguments={"value": 2}),
            ],
            2,
        ),
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-3", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-4", name="summarize", arguments={"value": 99}),
            ],
            4,
        ),
        (
            _tool_call(call_id="prefix-1", name="prefetch", arguments={"value": 0}),
            [
                _tool_call(call_id="call-1", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-2", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-3", name="expand", arguments={"value": 3}),
                _tool_call(call_id="call-4", name="lookup", arguments={"value": 1}),
                _tool_call(call_id="call-5", name="summarize", arguments={"value": 2}),
                _tool_call(call_id="call-6", name="expand", arguments={"value": 99}),
            ],
            6,
        ),
    ],
)
def test_detect_loop_rejects_near_misses(
    prefix_call: unified_llm.ToolCallData,
    near_miss_calls: list[unified_llm.ToolCallData],
    window: int,
) -> None:
    history = _history_from_tool_calls(near_miss_calls, prefix=[prefix_call])

    assert detect_loop(history, window=window) is False


@pytest.mark.asyncio
async def test_session_emits_loop_detection_warning_when_enabled() -> None:
    tool_call = _tool_call(call_id="call-1", name="lookup", arguments={"value": 1})
    client = _FakeCompleteClient(
        [
            _tool_call_response(
                response_id="resp-1",
                text="Need tool",
                tool_call=tool_call,
            ),
            _tool_call_response(
                response_id="resp-2",
                text="Need tool again",
                tool_call=_tool_call(
                    call_id="call-2",
                    name="lookup",
                    arguments={"value": 1},
                ),
            ),
            _final_response(response_id="resp-3", text="All done"),
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
        config=agent.SessionConfig(loop_detection_window=2),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    assert len(client.requests) == 3

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}

    first_assistant_start = await _next_event(stream)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert first_assistant_start.data == {"response_id": "resp-1"}
    first_assistant_delta = await _next_event(stream)
    assert first_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_assistant_delta.data == {
        "response_id": "resp-1",
        "delta": "Need tool",
    }
    first_assistant_end = await _next_event(stream)
    assert first_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert first_assistant_end.data == {"text": "Need tool", "reasoning": None}
    first_tool_start = await _next_event(stream)
    assert first_tool_start.kind == agent.EventKind.TOOL_CALL_START
    assert first_tool_start.data == {"tool_call_id": "call-1", "tool_name": "lookup"}
    first_tool_end = await _next_event(stream)
    assert first_tool_end.kind == agent.EventKind.TOOL_CALL_END
    assert first_tool_end.data == {
        "tool_call_id": "call-1",
        "tool_name": "lookup",
        "output": "tool result",
    }

    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert second_assistant_start.data == {"response_id": "resp-2"}
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "Need tool again",
    }
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {
        "text": "Need tool again",
        "reasoning": None,
    }
    second_tool_start = await _next_event(stream)
    assert second_tool_start.kind == agent.EventKind.TOOL_CALL_START
    assert second_tool_start.data == {"tool_call_id": "call-2", "tool_name": "lookup"}
    second_tool_end = await _next_event(stream)
    assert second_tool_end.kind == agent.EventKind.TOOL_CALL_END
    assert second_tool_end.data == {
        "tool_call_id": "call-2",
        "tool_name": "lookup",
        "output": "tool result",
    }

    loop_detection_event = await _next_event(stream)
    assert loop_detection_event.kind == agent.EventKind.LOOP_DETECTION
    assert loop_detection_event.data == {"message": LOOP_DETECTION_WARNING}

    final_assistant_start = await _next_event(stream)
    assert final_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert final_assistant_start.data == {"response_id": "resp-3"}
    final_assistant_delta = await _next_event(stream)
    assert final_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert final_assistant_delta.data == {
        "response_id": "resp-3",
        "delta": "All done",
    }
    final_assistant_end = await _next_event(stream)
    assert final_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert final_assistant_end.data == {"text": "All done", "reasoning": None}
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "SteeringTurn",
        "AssistantTurn",
    ]
    assert session.history[-2].content == LOOP_DETECTION_WARNING
    assert session.history[-1].response_id == "resp-3"
    assert session.history[-1].finish_reason.reason == "stop"


@pytest.mark.asyncio
async def test_session_skips_loop_detection_when_disabled() -> None:
    client = _FakeCompleteClient(
        [
            _tool_call_response(
                response_id="resp-1",
                text="Need tool",
                tool_call=_tool_call(
                    call_id="call-1",
                    name="lookup",
                    arguments={"value": 1},
                ),
            ),
            _tool_call_response(
                response_id="resp-2",
                text="Need tool again",
                tool_call=_tool_call(
                    call_id="call-2",
                    name="lookup",
                    arguments={"value": 1},
                ),
            ),
            _final_response(response_id="resp-3", text="All done"),
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
        config=agent.SessionConfig(enable_loop_detection=False, loop_detection_window=2),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    assert len(client.requests) == 3

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    first_assistant_start = await _next_event(stream)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    await _next_event(stream)
    await _next_event(stream)
    first_tool_start = await _next_event(stream)
    assert first_tool_start.kind == agent.EventKind.TOOL_CALL_START
    await _next_event(stream)

    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    await _next_event(stream)
    await _next_event(stream)
    second_tool_start = await _next_event(stream)
    assert second_tool_start.kind == agent.EventKind.TOOL_CALL_START
    second_tool_end = await _next_event(stream)
    assert second_tool_end.kind == agent.EventKind.TOOL_CALL_END

    final_assistant_start = await _next_event(stream)
    assert final_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert final_assistant_start.data == {"response_id": "resp-3"}
    await _next_event(stream)
    await _next_event(stream)
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END

    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
    ]
    assert not any(
        isinstance(turn, agent.SteeringTurn) and turn.content == LOOP_DETECTION_WARNING
        for turn in session.history
    )


@pytest.mark.asyncio
async def test_session_ignores_near_miss_tool_call_patterns() -> None:
    client = _FakeCompleteClient(
        [
            _tool_call_response(
                response_id="resp-1",
                text="Need tool",
                tool_call=_tool_call(
                    call_id="call-1",
                    name="lookup",
                    arguments={"value": 1},
                ),
            ),
            _tool_call_response(
                response_id="resp-2",
                text="Need tool again",
                tool_call=_tool_call(
                    call_id="call-2",
                    name="summarize",
                    arguments={"value": 2},
                ),
            ),
            _tool_call_response(
                response_id="resp-3",
                text="Need tool more",
                tool_call=_tool_call(
                    call_id="call-3",
                    name="lookup",
                    arguments={"value": 1},
                ),
            ),
            _tool_call_response(
                response_id="resp-4",
                text="Need tool still",
                tool_call=_tool_call(
                    call_id="call-4",
                    name="summarize",
                    arguments={"value": 99},
                ),
            ),
            _final_response(response_id="resp-5", text="All done"),
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
    tool_registry.register(
        agent.ToolDefinition(
            name="summarize",
            description="Summarize value",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, environment: "tool result",
    )
    session = agent.Session(
        profile=profile,
        llm_client=client,
        config=agent.SessionConfig(loop_detection_window=4),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Question")

    assert len(client.requests) == 5

    observed_kinds: list[agent.EventKind] = []
    while True:
        event = await _next_event(stream)
        observed_kinds.append(event.kind)
        if event.kind == agent.EventKind.PROCESSING_END:
            break

    assert agent.EventKind.LOOP_DETECTION not in observed_kinds
    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
    ]
    assert not any(isinstance(turn, agent.SteeringTurn) for turn in session.history)
