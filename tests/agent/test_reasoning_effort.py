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


@pytest.mark.asyncio
async def test_session_process_input_uses_current_reasoning_effort_and_provider_options() -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("First reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-2",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Second reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-3",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Third reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-4",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Fourth reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
        ]
    )
    profile = _PromptProfile(
        id="fake-provider",
        model="fake-model",
        provider_options_map={"temperature": 0.2},
    )
    session = agent.Session(
        profile=profile,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort="high"),
    )
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    await session.process_input("First input")
    assert client.requests[0].reasoning_effort == "high"
    assert client.requests[0].provider_options == {
        "fake-provider": {"temperature": 0.2}
    }
    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None

    session.config.reasoning_effort = "low"
    profile.provider_options_map = {"temperature": 0.3}

    await session.process_input("Second input")
    assert client.requests[1].reasoning_effort == "low"
    assert client.requests[1].provider_options == {
        "fake-provider": {"temperature": 0.3}
    }
    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None

    session.config.reasoning_effort = "medium"
    profile.provider_options_map = {"temperature": 0.4}

    await session.process_input("Third input")
    assert client.requests[2].reasoning_effort == "medium"
    assert client.requests[2].provider_options == {
        "fake-provider": {"temperature": 0.4}
    }
    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None

    session.config.reasoning_effort = None
    profile.provider_options_map = {"temperature": 0.5}

    await session.process_input("Fourth input")
    assert client.requests[3].reasoning_effort is None
    assert client.requests[3].provider_options == {
        "fake-provider": {"temperature": 0.5}
    }
    assert session.state == agent.SessionState.IDLE
    assert session.pending_question is None

    assert [request.reasoning_effort for request in client.requests] == [
        "high",
        "low",
        "medium",
        None,
    ]
    assert [request.provider_options for request in client.requests] == [
        {"fake-provider": {"temperature": 0.2}},
        {"fake-provider": {"temperature": 0.3}},
        {"fake-provider": {"temperature": 0.4}},
        {"fake-provider": {"temperature": 0.5}},
    ]
    assert [turn.text for turn in session.history] == [
        "First input",
        "First reply",
        "Second input",
        "Second reply",
        "Third input",
        "Third reply",
        "Fourth input",
        "Fourth reply",
    ]
    assert client.requests[0].reasoning_effort == "high"
    assert client.requests[1].reasoning_effort == "low"
    assert client.requests[2].reasoning_effort == "medium"
    assert client.requests[3].reasoning_effort is None
