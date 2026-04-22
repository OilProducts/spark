from __future__ import annotations

import asyncio
from collections.abc import AsyncIterable
from pathlib import Path
from uuid import UUID

import pytest

import unified_llm.agent as agent
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
        "ExecutionEnvironment",
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


def test_session_construction_exposes_public_state_and_queues() -> None:
    profile = agent.ProviderProfile(id="fake-provider", model="fake-model")
    environment = agent.ExecutionEnvironment(working_directory=".")
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
    session = agent.Session()
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
    assert first_user_input.data == {"content": "Answer one"}
    assert session.pending_question is None
    assert session.state == agent.SessionState.PROCESSING
    assert [turn.text for turn in session.history] == ["Answer one"]

    session.mark_natural_completion()
    first_processing_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert first_processing_end.kind == agent.EventKind.PROCESSING_END
    assert first_processing_end.data == {"state": "idle"}
    assert session.state == agent.SessionState.IDLE

    await session.submit("Answer two")
    second_user_input = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_user_input.kind == agent.EventKind.USER_INPUT
    assert second_user_input.data == {"content": "Answer two"}
    assert [turn.text for turn in session.history] == ["Answer one", "Answer two"]
    assert session.state == agent.SessionState.PROCESSING

    session.mark_turn_limit(round_count=1, total_turns=2)
    turn_limit_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert turn_limit_event.kind == agent.EventKind.TURN_LIMIT
    assert turn_limit_event.data == {
        "state": "idle",
        "round_count": 1,
        "total_turns": 2,
    }
    second_processing_end = await asyncio.wait_for(anext(stream), timeout=1)
    assert second_processing_end.kind == agent.EventKind.PROCESSING_END
    assert second_processing_end.data == {"state": "idle"}
    assert session.state == agent.SessionState.IDLE


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
        metadata={"kind": "builtin"},
    )
    registry = agent.ToolRegistry({"lookup": registered_tool})
    environment = agent.ExecutionEnvironment(
        working_directory=".",
        platform="posix",
        os_version="1.0",
        metadata={"shell": "bash"},
    )
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
    assert environment.working_directory == Path(".")
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
