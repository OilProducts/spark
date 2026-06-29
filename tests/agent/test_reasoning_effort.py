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


@pytest.mark.parametrize(
    ("profile", "expected_options"),
    [
        (
            agent.create_openai_profile(
                model="gpt-5.2",
                provider_options_map={"reasoning": {"summary": "auto"}},
            ),
            [
                {"openai": {"reasoning": {"summary": "auto", "effort": "low"}}},
                {"openai": {"reasoning": {"summary": "auto", "effort": "high"}}},
            ],
        ),
        (
            agent.create_anthropic_profile(
                model="claude-sonnet-4-5",
                provider_options_map={
                    "beta_headers": ["prompt-caching-2024-07-31"],
                    "thinking": {"budget_tokens": 128},
                },
            ),
            [
                {
                    "anthropic": {
                        "beta_headers": ["prompt-caching-2024-07-31"],
                        "thinking": {
                            "budget_tokens": 128,
                        },
                        "output_config": {
                            "effort": "low",
                        },
                    }
                },
                {
                    "anthropic": {
                        "beta_headers": ["prompt-caching-2024-07-31"],
                        "thinking": {
                            "budget_tokens": 128,
                        },
                        "output_config": {
                            "effort": "high",
                        },
                    }
                },
            ],
        ),
        (
            agent.create_gemini_profile(
                model="gemini-3.1-pro-preview",
                provider_options_map={
                    "topK": 32,
                    "thinkingConfig": {"includeThoughts": True},
                },
            ),
            [
                {
                    "gemini": {
                        "topK": 32,
                        "thinkingConfig": {
                            "includeThoughts": True,
                            "thinkingLevel": "low",
                        },
                    }
                },
                {
                    "gemini": {
                        "topK": 32,
                        "thinkingConfig": {
                            "includeThoughts": True,
                            "thinkingLevel": "high",
                        },
                    }
                },
            ],
        ),
    ],
    ids=["openai", "anthropic", "gemini"],
)
@pytest.mark.asyncio
async def test_native_profiles_update_reasoning_provider_options_between_model_calls(
    profile: agent.ProviderProfile,
    expected_options: list[dict[str, object]],
) -> None:
    client = _FakeCompleteClient(
        [
            unified_llm.Response(
                id="resp-1",
                model=profile.model,
                provider=profile.id,
                message=unified_llm.Message.assistant("First reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
            unified_llm.Response(
                id="resp-2",
                model=profile.model,
                provider=profile.id,
                message=unified_llm.Message.assistant("Second reply"),
                finish_reason=unified_llm.FinishReason.STOP,
            ),
        ]
    )
    session = agent.Session(
        profile=profile,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort="low"),
    )
    stream = session.events()
    assert (await asyncio.wait_for(anext(stream), timeout=1)).kind == (
        agent.EventKind.SESSION_START
    )

    await session.process_input("First input")
    session.config.reasoning_effort = "high"
    await session.process_input("Second input")

    assert [request.reasoning_effort for request in client.requests] == ["low", "high"]
    assert [request.provider_options for request in client.requests] == expected_options
    assert [turn.text for turn in session.history] == [
        "First input",
        "First reply",
        "Second input",
        "Second reply",
    ]


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
