from __future__ import annotations

import asyncio

import pytest

import agent
import unified_llm


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


@pytest.mark.asyncio
async def test_check_context_usage_emits_a_warning_when_history_exceeds_the_threshold(
    tmp_path,
) -> None:
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        context_window_size=100,
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    session.history = [agent.UserTurn(content="x" * 400)]
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    assert agent.check_context_usage(session) is True

    warning_event = await _next_event(stream)
    assert warning_event.kind == agent.EventKind.WARNING
    assert warning_event.data == {"message": "Context usage at ~100% of context window"}


@pytest.mark.asyncio
async def test_check_context_usage_stays_quiet_at_the_80_percent_boundary(tmp_path) -> None:
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        context_window_size=100,
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    session.history = [agent.UserTurn(content="x" * 320)]
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    assert agent.check_context_usage(session) is False
    assert session.event_queue.empty()


@pytest.mark.asyncio
async def test_context_warning_is_emitted_after_tool_results_push_history_over_threshold(
    tmp_path,
) -> None:
    tool_registry = agent.ToolRegistry()
    profile = _PromptProfile(
        id="fake-provider",
        model="fake-model",
        context_window_size=100,
        tool_registry=tool_registry,
    )
    tool_registry.register(
        agent.ToolDefinition(
            name="lookup",
            description="Lookup value",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, environment: "x" * 392,
    )
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
                            text="tool",
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
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("done"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
        ]
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("user")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "user"}
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {
        "response_id": "resp-1",
        "delta": "tool",
    }
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {"text": "tool", "reasoning": None, "response_id": "resp-1"}
    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    warning_event = await _next_event(stream)
    assert warning_event.kind == agent.EventKind.WARNING
    assert warning_event.data == {"message": "Context usage at ~100% of context window"}
    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "done",
    }
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END

    assert len(client.requests) == 2
    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "AssistantTurn",
        "ToolResultsTurn",
        "AssistantTurn",
    ]
