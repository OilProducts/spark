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


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


class _StrictQuestionProfile(_PromptProfile):
    def classify_text_completion(self, response_text: str) -> bool:
        return response_text.startswith("Need")


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


@pytest.mark.asyncio
async def test_session_process_input_pauses_on_question_and_delays_follow_ups() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Need more info?\n"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Here is the answer"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-3",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Queued follow-up handled"),
                finish_reason=unified_llm.FinishReason.STOP,
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

    await session.process_input("Initial input")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Initial input"}
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {
        "response_id": "resp-1",
        "delta": "Need more info?\n",
    }
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {
        "text": "Need more info?\n",
        "reasoning": None,
    }
    assert session.state == agent.SessionState.AWAITING_INPUT
    assert session.pending_question == "Need more info?\n"

    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(anext(stream), timeout=0.05)

    session.follow_up("Queued follow-up")
    assert session.follow_up_queue.qsize() == 1

    await session.process_input("Yes please")

    answer_event = await _next_event(stream)
    assert answer_event.kind == agent.EventKind.USER_INPUT
    assert answer_event.data == {
        "content": "Yes please",
        "answer_to": "Need more info?\n",
    }
    answer_assistant_start = await _next_event(stream)
    assert answer_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    answer_assistant_delta = await _next_event(stream)
    assert answer_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert answer_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "Here is the answer",
    }
    answer_assistant_end = await _next_event(stream)
    assert answer_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert answer_assistant_end.data == {
        "text": "Here is the answer",
        "reasoning": None,
    }
    follow_up_event = await _next_event(stream)
    assert follow_up_event.kind == agent.EventKind.USER_INPUT
    assert follow_up_event.data == {"content": "Queued follow-up"}
    follow_up_assistant_start = await _next_event(stream)
    assert follow_up_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    follow_up_assistant_delta = await _next_event(stream)
    assert follow_up_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert follow_up_assistant_delta.data == {
        "response_id": "resp-3",
        "delta": "Queued follow-up handled",
    }
    follow_up_assistant_end = await _next_event(stream)
    assert follow_up_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert follow_up_assistant_end.data == {
        "text": "Queued follow-up handled",
        "reasoning": None,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None
    assert session.follow_up_queue.qsize() == 0
    assert [turn.text for turn in session.history] == [
        "Initial input",
        "Need more info?\n",
        "Yes please",
        "Here is the answer",
        "Queued follow-up",
        "Queued follow-up handled",
    ]
    assert len(client.requests) == 3
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Initial input"),
    ]
    assert client.requests[1].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:3]),
    ]
    assert client.requests[2].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:5]),
    ]


@pytest.mark.asyncio
async def test_session_process_input_replays_follow_ups_after_natural_completion() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Initial reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("First queued reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-3",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Second queued reply"),
                finish_reason=unified_llm.FinishReason.STOP,
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

    session.follow_up("First queued follow-up")
    session.follow_up("Second queued follow-up")

    await session.process_input("Initial input")

    first_user_input = await _next_event(stream)
    assert first_user_input.kind == agent.EventKind.USER_INPUT
    assert first_user_input.data == {"content": "Initial input"}
    first_assistant_start = await _next_event(stream)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    first_assistant_delta = await _next_event(stream)
    assert first_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_assistant_delta.data == {
        "response_id": "resp-1",
        "delta": "Initial reply",
    }
    first_assistant_end = await _next_event(stream)
    assert first_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert first_assistant_end.data == {
        "text": "Initial reply",
        "reasoning": None,
    }
    first_follow_up = await _next_event(stream)
    assert first_follow_up.kind == agent.EventKind.USER_INPUT
    assert first_follow_up.data == {"content": "First queued follow-up"}
    second_assistant_start = await _next_event(stream)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    second_assistant_delta = await _next_event(stream)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "First queued reply",
    }
    second_assistant_end = await _next_event(stream)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {
        "text": "First queued reply",
        "reasoning": None,
    }
    second_follow_up = await _next_event(stream)
    assert second_follow_up.kind == agent.EventKind.USER_INPUT
    assert second_follow_up.data == {"content": "Second queued follow-up"}
    third_assistant_start = await _next_event(stream)
    assert third_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    third_assistant_delta = await _next_event(stream)
    assert third_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert third_assistant_delta.data == {
        "response_id": "resp-3",
        "delta": "Second queued reply",
    }
    third_assistant_end = await _next_event(stream)
    assert third_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert third_assistant_end.data == {
        "text": "Second queued reply",
        "reasoning": None,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None
    assert [turn.text for turn in session.history] == [
        "Initial input",
        "Initial reply",
        "First queued follow-up",
        "First queued reply",
        "Second queued follow-up",
        "Second queued reply",
    ]
    assert len(client.requests) == 3
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Initial input"),
    ]
    assert client.requests[1].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:3]),
    ]
    assert client.requests[2].messages == [
        unified_llm.Message.system("Session system prompt"),
        *history_to_messages(session.history[:5]),
    ]


@pytest.mark.asyncio
async def test_session_process_input_drains_steering_in_fifo_order() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Assistant reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            )
        ]
    )
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    session.steer("First steering message")
    session.steer("Second steering message")

    await session.process_input("Initial input")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Initial input"}
    first_steering_event = await _next_event(stream)
    assert first_steering_event.kind == agent.EventKind.STEERING_INJECTED
    assert first_steering_event.data == {"content": "First steering message"}
    second_steering_event = await _next_event(stream)
    assert second_steering_event.kind == agent.EventKind.STEERING_INJECTED
    assert second_steering_event.data == {"content": "Second steering message"}
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {
        "response_id": "resp-1",
        "delta": "Assistant reply",
    }
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {
        "text": "Assistant reply",
        "reasoning": None,
    }
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in session.history] == [
        "UserTurn",
        "SteeringTurn",
        "SteeringTurn",
        "AssistantTurn",
    ]
    assert [turn.text for turn in session.history] == [
        "Initial input",
        "First steering message",
        "Second steering message",
        "Assistant reply",
    ]
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Initial input"),
        unified_llm.Message.user("First steering message"),
        unified_llm.Message.user("Second steering message"),
    ]


@pytest.mark.asyncio
async def test_session_process_input_allows_profile_text_completion_override() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Need anything else"),
                finish_reason=unified_llm.FinishReason.STOP,
            )
        ]
    )
    session = agent.Session(
        profile=_StrictQuestionProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Initial input")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assistant_start_event = await _next_event(stream)
    assert assistant_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_delta_event = await _next_event(stream)
    assert assistant_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert assistant_delta_event.data == {
        "response_id": "resp-1",
        "delta": "Need anything else",
    }
    assistant_end_event = await _next_event(stream)
    assert assistant_end_event.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert assistant_end_event.data == {
        "text": "Need anything else",
        "reasoning": None,
    }
    assert session.state == agent.SessionState.AWAITING_INPUT
    assert session.pending_question == "Need anything else"

    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(anext(stream), timeout=0.05)

    assert len(client.requests) == 1
    assert client.requests[0].messages == [
        unified_llm.Message.system("Session system prompt"),
        unified_llm.Message.user("Initial input"),
    ]
