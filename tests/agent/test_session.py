from __future__ import annotations

import asyncio
import importlib
from collections.abc import AsyncIterable
from pathlib import Path
from uuid import UUID

import pytest

import agent
import unified_llm
from unified_llm import (
    Client,
    ContentKind,
    ContentPart,
    FinishReason,
    ToolCallData,
    ToolResultData,
    Usage,
)


def test_agent_package_re_exports_the_public_foundation_types() -> None:
    expected_names = {
        "AgentError",
        "AssistantTurn",
        "DirEntry",
        "ExecutionEnvironment",
        "EnvironmentInheritancePolicy",
        "ExecResult",
        "GrepOptions",
        "LocalExecutionEnvironment",
        "ProviderProfile",
        "RegisteredTool",
        "Session",
        "SessionAbortedError",
        "SessionClosedError",
        "SessionConfig",
        "SessionState",
        "SessionStateError",
        "SteeringTurn",
        "SubAgentError",
        "SubAgentHandle",
        "SubAgentLimitError",
        "SubAgentResult",
        "SubAgentStatus",
        "SystemTurn",
        "ToolDefinition",
        "ToolRegistry",
        "ToolResultsTurn",
        "UserTurn",
    }

    assert expected_names <= set(agent.__all__)
    for name in expected_names:
        assert hasattr(agent, name)


def test_legacy_unified_llm_agent_namespace_is_not_supported() -> None:
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module("unified_llm.agent")


def test_session_construction_exposes_public_state_and_queues() -> None:
    profile = agent.ProviderProfile(id="fake-provider", model="fake-model")
    environment = agent.LocalExecutionEnvironment(working_dir=".")
    client = Client()
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
    )

    assert isinstance(session.id, UUID)
    assert isinstance(session.provider_profile, agent.ProviderProfile)
    assert isinstance(session.execution_environment, agent.ExecutionEnvironment)
    assert isinstance(session.config, agent.SessionConfig)
    assert session.state == agent.SessionState.IDLE
    assert session.history == []
    assert isinstance(session.event_queue, asyncio.Queue)
    assert isinstance(session.steering_queue, asyncio.Queue)
    assert isinstance(session.follow_up_queue, asyncio.Queue)
    assert session.active_subagents == {}
    assert session.profile is session.provider_profile
    assert session.environment is session.execution_environment
    assert session.execution_env is session.execution_environment
    assert session.session_id == session.id
    assert session.client is client
    assert session.llm_client is client
    assert session.event_emitter is session.event_queue
    assert session.followup_queue is session.follow_up_queue
    assert session.subagents is session.active_subagents
    assert session.pending_question is None
    assert session.abort_signaled is False


def test_session_events_is_public_and_queue_backed() -> None:
    session = agent.Session()

    assert callable(session.events)
    assert isinstance(session.events(), AsyncIterable)
    assert isinstance(session.event_stream, AsyncIterable)


@pytest.mark.asyncio
async def test_session_process_input_submit_and_state_helpers_are_public() -> None:
    class _FakeClient:
        def __init__(self) -> None:
            self.requests: list[unified_llm.Request] = []
            self.responses = [
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
                                thinking=unified_llm.ThinkingData(
                                    text="Assistant thinking",
                                ),
                                text="Assistant thinking",
                            ),
                        ]
                    ),
                    finish_reason=unified_llm.FinishReason.STOP,
                    usage=unified_llm.Usage(
                        input_tokens=3,
                        output_tokens=5,
                        total_tokens=8,
                    ),
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
                                thinking=unified_llm.ThinkingData(
                                    text="Second thinking",
                                ),
                                text="Second thinking",
                            ),
                        ]
                    ),
                    finish_reason=unified_llm.FinishReason.STOP,
                    usage=unified_llm.Usage(
                        input_tokens=4,
                        output_tokens=6,
                        total_tokens=10,
                    ),
                ),
            ]

        async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
            self.requests.append(request)
            return self.responses.pop(0)

    client = _FakeClient()
    profile = agent.ProviderProfile(id="fake-provider", model="fake-model")
    session = agent.Session(profile=profile, llm_client=client)
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    session.mark_awaiting_input("What is the next step?")
    assert session.state == agent.SessionState.AWAITING_INPUT
    assert session.pending_question == "What is the next step?"

    await session.process_input("Answer one")
    first_user_input = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_user_input.kind == agent.EventKind.USER_INPUT
    assert first_user_input.data == {
        "content": "Answer one",
        "answer_to": "What is the next step?",
    }
    assert session.pending_question is None
    assert session.state == agent.SessionState.IDLE
    first_assistant_start = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert first_assistant_start.data == {"response_id": "resp-1"}
    first_assistant_delta = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert first_assistant_delta.data == {
        "response_id": "resp-1",
        "delta": "Assistant reply",
    }
    first_assistant_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert first_assistant_end.data == {
        "text": "Assistant reply",
        "reasoning": "Assistant thinking",
    }
    first_processing_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_processing_end.kind == agent.EventKind.PROCESSING_END
    assert first_processing_end.data == {"state": "idle"}
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
    second_user_input = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_user_input.kind == agent.EventKind.USER_INPUT
    assert second_user_input.data == {
        "content": "Answer two",
        "answer_to": "And then?",
    }
    assert session.pending_question is None
    assert session.state == agent.SessionState.IDLE
    second_assistant_start = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_assistant_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assert second_assistant_start.data == {"response_id": "resp-2"}
    second_assistant_delta = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_assistant_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert second_assistant_delta.data == {
        "response_id": "resp-2",
        "delta": "Second reply",
    }
    second_assistant_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_assistant_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    assert second_assistant_end.data == {
        "text": "Second reply",
        "reasoning": "Second thinking",
    }
    second_processing_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_processing_end.kind == agent.EventKind.PROCESSING_END
    assert second_processing_end.data == {"state": "idle"}
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


@pytest.mark.asyncio
async def test_session_abort_marks_closed_and_emits_session_end() -> None:
    session = agent.Session()
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.abort()

    end_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}
    assert session.state == agent.SessionState.CLOSED
    assert session.abort_signaled is True


@pytest.mark.asyncio
async def test_session_unrecoverable_error_emits_error_and_closes() -> None:
    session = agent.Session()
    stream = session.events()

    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.mark_unrecoverable_error(RuntimeError("boom"))

    error_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert error_event.kind == agent.EventKind.ERROR
    assert error_event.data == {"error": "boom"}

    end_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}
    assert session.state == agent.SessionState.CLOSED


def test_session_config_defaults_and_aliases_are_public() -> None:
    config = agent.SessionConfig(
        tool_output_limits={"shell": 120},
        line_limits={"shell": 4},
    )

    assert config.max_turns == 0
    assert config.max_tool_rounds_per_input == 0
    assert config.default_command_timeout_ms == 10000
    assert config.max_command_timeout_ms == 600000
    assert config.reasoning_effort is None
    assert config.tool_output_limits == {"shell": 120}
    assert config.line_limits == {"shell": 4}
    assert config.tool_output_char_limits == {"shell": 120}
    assert config.tool_line_limits == {"shell": 4}
    assert config.enable_loop_detection is True
    assert config.loop_detection_window == 10
    assert config.max_subagent_depth == 1

    config.tool_output_char_limits = {"stdout": 256}
    config.tool_line_limits = {"stdout": 8}

    assert config.tool_output_limits == {"stdout": 256}
    assert config.line_limits == {"stdout": 8}


def test_session_state_exports_cover_the_required_values() -> None:
    assert [member.name for member in agent.SessionState] == [
        "IDLE",
        "PROCESSING",
        "AWAITING_INPUT",
        "CLOSED",
    ]
    assert [member.value for member in agent.SessionState] == [
        "idle",
        "processing",
        "awaiting_input",
        "closed",
    ]


def test_turn_and_foundation_records_are_constructible_through_public_imports() -> None:
    text_part = ContentPart(kind=ContentKind.TEXT, text="hello")
    tool_call = ToolCallData(id="call-1", name="lookup", arguments={"value": 1})
    tool_result = ToolResultData(tool_call_id="call-1", content="done", is_error=False)
    usage = Usage(input_tokens=3, output_tokens=5, total_tokens=8)

    user_turn = agent.UserTurn(content=[text_part])
    system_turn = agent.SystemTurn(content="system note")
    steering_turn = agent.SteeringTurn(content="steer")
    assistant_turn = agent.AssistantTurn(
        content=[text_part],
        tool_calls=[tool_call],
        reasoning="thinking",
        usage=usage,
        response_id="resp-1",
        finish_reason=FinishReason.STOP,
    )
    tool_results_turn = agent.ToolResultsTurn(result_list=[tool_result])

    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup value",
        parameters={"type": "object"},
        metadata={"source": "test"},
    )
    registered_tool = agent.RegisteredTool(
        definition=tool_definition,
        executor=lambda arguments, execution_environment: "done",
        metadata={"kind": "builtin"},
    )
    registry = agent.ToolRegistry({"lookup": registered_tool})
    environment = agent.LocalExecutionEnvironment(working_dir=".")
    profile = agent.ProviderProfile(
        id="provider-id",
        model="model-name",
        tool_registry=registry,
        capabilities={"tool_calls": True},
        provider_options_map={"temperature": 0.2},
        context_window_size=4096,
        display_name="Test Provider",
    )
    handle = agent.SubAgentHandle(
        id="12345678-1234-1234-1234-123456789abc",
        status=agent.SubAgentStatus.RUNNING,
        session_id="87654321-4321-4321-4321-cba987654321",
        provider_profile=profile,
        working_directory=".",
        metadata={"scope": "child"},
    )
    result = agent.SubAgentResult(
        handle_id=handle.id,
        status=agent.SubAgentStatus.COMPLETED,
        session_id=handle.session_id,
        turns=[agent.UserTurn(content="done")],
        response_id="response-1",
        summary="complete",
        metadata={"tokens": 4},
    )

    assert user_turn.content == [text_part]
    assert system_turn.content == "system note"
    assert steering_turn.content == "steer"
    assert assistant_turn.content == [text_part]
    assert assistant_turn.tool_calls == [tool_call]
    assert assistant_turn.reasoning == "thinking"
    assert assistant_turn.usage == usage
    assert assistant_turn.response_id == "resp-1"
    assert assistant_turn.finish_reason.reason == "stop"
    assert tool_results_turn.result_list == [tool_result]
    assert tool_results_turn.results == [tool_result]

    for turn in (user_turn, system_turn, steering_turn, assistant_turn, tool_results_turn):
        assert turn.timestamp.tzinfo is not None

    assert registry.get("lookup") is registered_tool
    assert registry.definitions() == [tool_definition]
    assert environment.working_directory() == "."
    assert profile.tools() == [tool_definition]
    assert profile.provider_options() == {"temperature": 0.2}
    assert profile.supports("tool_calls") is True
    assert handle.profile is profile
    assert handle.working_directory == Path(".")
    assert handle.status == agent.SubAgentStatus.RUNNING
    assert result.handle_id == handle.id
    assert result.session_id == handle.session_id
    assert result.status == agent.SubAgentStatus.COMPLETED
    assert result.turns[0].text == "done"
    assert result.response_id == "response-1"
    assert result.summary == "complete"

    assert issubclass(agent.SessionClosedError, agent.SessionStateError)
    assert issubclass(agent.SessionAbortedError, agent.SessionStateError)
    assert issubclass(agent.SubAgentError, agent.AgentError)
    assert issubclass(agent.SubAgentLimitError, agent.SubAgentError)
