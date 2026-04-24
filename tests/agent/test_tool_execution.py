from __future__ import annotations

import asyncio
import logging

import pytest

import agent
import agent.builtin_tools as builtin_tools
import unified_llm


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def _make_builtin_session(
    tmp_path,
    *,
    environment: object | None = None,
    capabilities: dict[str, bool] | None = None,
) -> agent.Session:
    execution_environment = (
        environment
        if environment is not None
        else agent.LocalExecutionEnvironment(working_dir=tmp_path)
    )
    profile = agent.ProviderProfile(
        id="builtin-provider",
        model="builtin-model",
        capabilities=capabilities or {},
    )
    registry = agent.ToolRegistry()
    builtin_tools.register_builtin_tools(registry, provider_profile=profile)
    profile.tool_registry = registry
    return agent.Session(profile=profile, execution_env=execution_environment)


def _make_subagent_session(
    tmp_path,
    *,
    client: object | None = None,
    max_subagent_depth: int = 1,
) -> agent.Session:
    profile = agent.ProviderProfile(
        id="subagent-provider",
        model="subagent-model",
        capabilities={"tool_calls": True},
    )
    profile.tool_registry = agent.build_subagent_tool_registry()
    return agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
        llm_client=client,
        config=agent.SessionConfig(max_subagent_depth=max_subagent_depth),
    )


def _assistant_response(text: str, response_id: str) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="subagent-model",
        provider="subagent-provider",
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason.STOP,
    )


class _BlockingCompleteClient:
    def __init__(
        self,
        responses: list[unified_llm.Response],
        *,
        errors: list[BaseException | None] | None = None,
    ) -> None:
        self.requests: list[unified_llm.Request] = []
        self.responses = list(responses)
        self.errors = list(errors or [None] * len(responses))
        self.started: list[asyncio.Event] = [asyncio.Event() for _ in responses]
        self.released: list[asyncio.Event] = [asyncio.Event() for _ in responses]

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        index = len(self.requests)
        if index >= len(self.responses):
            raise AssertionError("unexpected complete call")
        self.requests.append(request)
        self.started[index].set()
        await self.released[index].wait()
        error = self.errors[index]
        if error is not None:
            raise error
        return self.responses[index]


@pytest.mark.asyncio
async def test_execute_tool_call_truncates_model_result_but_keeps_full_event_output(
    tmp_path,
) -> None:
    output = "0123456789ABCDEFGHIJ"
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Echo output",
        parameters={"type": "object"},
    )

    def executor(arguments: dict[str, object], execution_environment: object) -> str:
        assert arguments == {}
        assert execution_environment is expected_environment
        return output

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
        config=agent.SessionConfig(tool_output_limits={"shell": 10}),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-1", name="shell", arguments="{}"),
    )

    assert result.tool_call_id == "call-1"
    assert result.is_error is False
    assert result.content == agent.truncate_output(output, 10, "head_tail")

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert tool_start_event.data == {"tool_call_id": "call-1", "tool_name": "shell"}

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-1",
        "tool_name": "shell",
        "output": output,
    }


@pytest.mark.asyncio
async def test_execute_tool_call_returns_recoverable_error_for_unknown_tools(
    tmp_path,
) -> None:
    session = agent.Session(
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-2", name="missing", arguments="{}"),
    )

    assert result.tool_call_id == "call-2"
    assert result.is_error is True
    assert result.content == "Unknown tool: missing"

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-2",
        "tool_name": "missing",
        "error": "Unknown tool: missing",
    }


@pytest.mark.asyncio
async def test_execute_tool_call_routes_registered_builtin_tools_through_the_pipeline(
    tmp_path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    session = _make_builtin_session(tmp_path, environment=environment)
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="call-read",
            name="read_file",
            arguments={"path": "missing.txt"},
        ),
    )

    assert result.tool_call_id == "call-read"
    assert result.is_error is True
    assert result.content == "File not found: missing.txt"

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert tool_start_event.data == {
        "tool_call_id": "call-read",
        "tool_name": "read_file",
    }

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data["tool_call_id"] == "call-read"
    assert tool_end_event.data["tool_name"] == "read_file"
    error = tool_end_event.data["error"]
    assert isinstance(error, agent.ToolOutput)
    assert error.content == "File not found: missing.txt"
    assert error.is_error is True


@pytest.mark.asyncio
async def test_execute_tool_call_rejects_invalid_json_arguments(tmp_path) -> None:
    called = False
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Echo output",
        parameters={"type": "object"},
    )

    def executor(arguments: dict[str, object], execution_environment: object) -> str:
        nonlocal called
        called = True
        return "unexpected"

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-json", name="shell", arguments="{not json"),
    )

    assert called is False
    assert result.tool_call_id == "call-json"
    assert result.is_error is True
    assert result.content.startswith("Invalid arguments for tool: shell")

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data["error"].startswith("Invalid arguments for tool: shell")


@pytest.mark.asyncio
async def test_execute_tool_call_rejects_schema_validation_failures(tmp_path) -> None:
    called = False
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup value",
        parameters={
            "type": "object",
            "properties": {"value": {"type": "integer"}},
            "required": ["value"],
            "additionalProperties": False,
        },
    )

    def executor(arguments: dict[str, object], execution_environment: object) -> str:
        nonlocal called
        called = True
        return "unexpected"

    registry = agent.ToolRegistry(
        {
            "lookup": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-schema", name="lookup", arguments="{}"),
    )

    assert called is False
    assert result.tool_call_id == "call-schema"
    assert result.is_error is True
    assert result.content.startswith("Invalid arguments for tool: lookup")

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data["error"].startswith("Invalid arguments for tool: lookup")


@pytest.mark.asyncio
async def test_execute_tool_call_preserves_sdk_tool_result_objects(tmp_path) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="structured",
        description="Return a structured result",
        parameters={"type": "object"},
    )
    tool_result = unified_llm.ToolResult(
        tool_call_id="call-structured",
        content={"answer": 42},
        is_error=False,
    )

    def executor(arguments: dict[str, object], execution_environment: object):
        assert arguments == {}
        assert execution_environment is expected_environment
        return tool_result

    registry = agent.ToolRegistry(
        {
            "structured": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-structured", name="structured", arguments="{}"),
    )

    assert result.tool_call_id == "call-structured"
    assert result.is_error is False
    assert result.content == {"answer": 42}

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-structured",
        "tool_name": "structured",
        "output": tool_result,
    }


@pytest.mark.asyncio
async def test_execute_tool_call_preserves_successful_exec_result_output(
    tmp_path,
) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Run a command",
        parameters={"type": "object"},
    )
    exec_result = agent.ExecResult(
        stdout="stdout",
        stderr="stderr",
        exit_code=0,
        timed_out=False,
        duration_ms=15,
    )

    def executor(arguments: dict[str, object], execution_environment: object):
        assert arguments == {}
        assert execution_environment is expected_environment
        return exec_result

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-exec-success", name="shell", arguments="{}"),
    )

    assert result.tool_call_id == "call-exec-success"
    assert result.is_error is False
    assert result.content == "stdout\nstderr"

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-exec-success",
        "tool_name": "shell",
        "output": exec_result,
    }


@pytest.mark.asyncio
async def test_execute_tool_call_passes_session_config_to_opt_in_executors(
    tmp_path,
) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Run a command",
        parameters={"type": "object"},
    )
    seen_session_config: agent.SessionConfig | None = None

    def executor(
        arguments: dict[str, object],
        execution_environment: object,
        session_config: agent.SessionConfig | None = None,
    ) -> str:
        nonlocal seen_session_config
        assert arguments == {}
        assert execution_environment is expected_environment
        seen_session_config = session_config
        return "ok"

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session_config = agent.SessionConfig(
        default_command_timeout_ms=123,
        max_command_timeout_ms=456,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
        config=session_config,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-config", name="shell", arguments="{}"),
    )

    assert seen_session_config is session_config
    assert result.tool_call_id == "call-config"
    assert result.is_error is False
    assert result.content == "ok"


@pytest.mark.asyncio
async def test_execute_tool_call_passes_parent_session_to_opt_in_executors(
    tmp_path,
) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )
    seen_session: agent.Session | None = None

    def executor(
        arguments: dict[str, object],
        execution_environment: object,
        session: agent.Session | None = None,
    ) -> str:
        nonlocal seen_session
        assert arguments == {}
        assert execution_environment is expected_environment
        seen_session = session
        return "ok"

    registry = agent.ToolRegistry(
        {
            "lookup": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(id="call-session", name="lookup", arguments="{}"),
    )

    assert seen_session is session
    assert result.tool_call_id == "call-session"
    assert result.is_error is False
    assert result.content == "ok"


@pytest.mark.asyncio
async def test_execute_tool_call_routes_subagent_tools_through_the_normal_pipeline(
    tmp_path,
) -> None:
    client = _BlockingCompleteClient(
        [
            _assistant_response("child response 1", "resp-1"),
            _assistant_response("child response 2", "resp-2"),
        ]
    )
    session = _make_subagent_session(tmp_path, client=client)
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )
    assert spawn_result.is_error is False
    agent_id = spawn_result.content["agent_id"]

    spawn_start_event = await _next_event(stream)
    assert spawn_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert spawn_start_event.data == {
        "tool_call_id": "spawn-1",
        "tool_name": "spawn_agent",
    }

    spawn_end_event = await _next_event(stream)
    assert spawn_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert spawn_end_event.data["tool_call_id"] == "spawn-1"
    assert spawn_end_event.data["tool_name"] == "spawn_agent"
    assert spawn_end_event.data["output"].content["status"] == "running"

    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    send_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="send-1",
            name="send_input",
            arguments={
                "agent_id": agent_id,
                "message": "Please continue",
            },
        ),
    )
    assert send_result.is_error is False
    assert send_result.content["status"] == "running"

    send_start_event = await _next_event(stream)
    assert send_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert send_start_event.data == {
        "tool_call_id": "send-1",
        "tool_name": "send_input",
    }

    send_end_event = await _next_event(stream)
    assert send_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert send_end_event.data["output"].content["status"] == "running"

    client.released[0].set()
    await asyncio.wait_for(client.started[1].wait(), timeout=1)

    wait_task = asyncio.create_task(
        agent.execute_tool_call(
            session,
            unified_llm.ToolCallData(
                id="wait-1",
                name="wait",
                arguments={"agent_id": agent_id},
            ),
        )
    )

    wait_start_event = await _next_event(stream)
    assert wait_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert wait_start_event.data == {
        "tool_call_id": "wait-1",
        "tool_name": "wait",
    }

    client.released[1].set()
    wait_result = await asyncio.wait_for(wait_task, timeout=1)
    assert wait_result.is_error is False
    assert wait_result.content["status"] == "completed"
    assert wait_result.content["success"] is True
    assert wait_result.content["output"] == "child response 2"
    assert wait_result.content["turns_used"] == 4

    wait_end_event = await _next_event(stream)
    assert wait_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert wait_end_event.data["output"].content["status"] == "completed"

    close_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="close-1",
            name="close_agent",
            arguments={"agent_id": agent_id},
        ),
    )
    assert close_result.is_error is False
    assert close_result.content["status"] == "completed"

    close_start_event = await _next_event(stream)
    assert close_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert close_start_event.data == {
        "tool_call_id": "close-1",
        "tool_name": "close_agent",
    }

    close_end_event = await _next_event(stream)
    assert close_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert close_end_event.data["output"].content["status"] == "completed"

    await session.close()


@pytest.mark.asyncio
async def test_execute_tool_call_returns_recoverable_error_for_subagent_startup_failure(
    tmp_path,
) -> None:
    session = _make_subagent_session(tmp_path, max_subagent_depth=0)
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-failure",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )

    assert result.tool_call_id == "spawn-failure"
    assert result.is_error is True
    assert "max_subagent_depth" in result.content

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data["tool_call_id"] == "spawn-failure"
    assert tool_end_event.data["tool_name"] == "spawn_agent"
    error = tool_end_event.data["error"]
    assert isinstance(error, agent.ToolOutput)
    assert "max_subagent_depth" in error.content

    await session.close()


@pytest.mark.asyncio
async def test_execute_tool_call_converts_failed_exec_result_into_recoverable_error(
    tmp_path,
    caplog,
) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Run a command",
        parameters={"type": "object"},
    )
    exec_result = agent.ExecResult(
        stdout="partial",
        stderr="boom",
        exit_code=7,
        timed_out=False,
        duration_ms=12,
    )

    def executor(arguments: dict[str, object], execution_environment: object):
        assert arguments == {}
        assert execution_environment is expected_environment
        return exec_result

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.WARNING):
        result = await agent.execute_tool_call(
            session,
            unified_llm.ToolCallData(id="call-exec-failure", name="shell", arguments="{}"),
        )

    assert result.tool_call_id == "call-exec-failure"
    assert result.is_error is True
    assert result.content == "partial\nboom\n[exit_code=7, timed_out=False, duration_ms=12]"
    assert any(
        record.levelno == logging.WARNING
        and "failed ExecResult" in record.message
        and "shell" in record.message
        for record in caplog.records
    )

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-exec-failure",
        "tool_name": "shell",
        "error": {
            "stdout": "partial",
            "stderr": "boom",
            "exit_code": 7,
            "timed_out": False,
            "duration_ms": 12,
        },
    }


@pytest.mark.asyncio
async def test_execute_tool_call_converts_timed_out_exec_result_into_recoverable_error(
    tmp_path,
    caplog,
) -> None:
    expected_environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Run a command",
        parameters={"type": "object"},
    )
    exec_result = agent.ExecResult(
        stdout="start",
        stderr="Command timed out after 50 ms",
        exit_code=124,
        timed_out=True,
        duration_ms=53,
    )

    def executor(arguments: dict[str, object], execution_environment: object):
        assert arguments == {}
        assert execution_environment is expected_environment
        return exec_result

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=expected_environment,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.WARNING):
        result = await agent.execute_tool_call(
            session,
            unified_llm.ToolCallData(id="call-exec-timeout", name="shell", arguments="{}"),
        )

    assert result.tool_call_id == "call-exec-timeout"
    assert result.is_error is True
    assert result.content == (
        "start\nCommand timed out after 50 ms\n"
        "[exit_code=124, timed_out=True, duration_ms=53]"
    )
    assert any(
        record.levelno == logging.WARNING
        and "failed ExecResult" in record.message
        and "shell" in record.message
        for record in caplog.records
    )

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-exec-timeout",
        "tool_name": "shell",
        "error": {
            "stdout": "start",
            "stderr": "Command timed out after 50 ms",
            "exit_code": 124,
            "timed_out": True,
            "duration_ms": 53,
        },
    }


@pytest.mark.asyncio
async def test_execute_tool_call_converts_executor_exceptions_into_recoverable_errors(
    tmp_path,
    caplog,
) -> None:
    tool_definition = agent.ToolDefinition(
        name="shell",
        description="Echo output",
        parameters={"type": "object"},
    )

    def executor(arguments: dict[str, object], execution_environment: object) -> str:
        raise PermissionError("denied")

    registry = agent.ToolRegistry(
        {
            "shell": agent.RegisteredTool(
                definition=tool_definition,
                executor=executor,
            )
        }
    )
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.ERROR):
        result = await agent.execute_tool_call(
            session,
            unified_llm.ToolCallData(id="call-exc", name="shell", arguments="{}"),
        )

    assert result.tool_call_id == "call-exc"
    assert result.is_error is True
    assert result.content == "Tool error (shell): denied"
    assert any(
        record.levelno >= logging.ERROR and "Tool execution failed for shell" in record.message
        for record in caplog.records
    )

    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START

    tool_end_event = await _next_event(stream)
    assert tool_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert tool_end_event.data == {
        "tool_call_id": "call-exc",
        "tool_name": "shell",
        "error": "Tool error (shell): denied",
    }


@pytest.mark.asyncio
async def test_execute_tool_calls_preserves_order_when_parallel_execution_is_supported(
    tmp_path,
) -> None:
    first_started = asyncio.Event()
    second_started = asyncio.Event()
    release_first = asyncio.Event()
    release_second = asyncio.Event()

    async def first_executor(
        arguments: dict[str, object],
        execution_environment: object,
    ) -> str:
        assert arguments == {}
        first_started.set()
        await release_first.wait()
        return "first"

    async def second_executor(
        arguments: dict[str, object],
        execution_environment: object,
    ) -> str:
        assert arguments == {}
        second_started.set()
        await release_second.wait()
        return "second"

    first_definition = agent.ToolDefinition(
        name="first",
        description="First executor",
        parameters={"type": "object"},
    )
    second_definition = agent.ToolDefinition(
        name="second",
        description="Second executor",
        parameters={"type": "object"},
    )
    registry = agent.ToolRegistry(
        {
            "first": agent.RegisteredTool(
                definition=first_definition,
                executor=first_executor,
            ),
            "second": agent.RegisteredTool(
                definition=second_definition,
                executor=second_executor,
            ),
        }
    )

    class ParallelProfile:
        def __init__(self, tool_registry: agent.ToolRegistry) -> None:
            self.id = "parallel-provider"
            self.model = "parallel-model"
            self.tool_registry = tool_registry
            self.supports_parallel_tool_calls = True

    session = agent.Session(
        profile=ParallelProfile(registry),
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    task = asyncio.create_task(
        agent.execute_tool_calls(
            session,
            [
                unified_llm.ToolCallData(id="call-first", name="first", arguments="{}"),
                unified_llm.ToolCallData(id="call-second", name="second", arguments="{}"),
            ],
        )
    )

    await asyncio.wait_for(first_started.wait(), timeout=1)
    await asyncio.wait_for(second_started.wait(), timeout=1)
    release_second.set()
    await asyncio.sleep(0)
    release_first.set()

    results = await asyncio.wait_for(task, timeout=1)

    assert [result.tool_call_id for result in results] == ["call-first", "call-second"]
    assert [result.content for result in results] == ["first", "second"]


@pytest.mark.asyncio
async def test_execute_tool_calls_preserves_order_for_registered_builtin_tools(
    tmp_path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("notes.txt", "alpha\nbeta\n")
    environment.write_file("nested/other.txt", "gamma\n")
    session = _make_builtin_session(tmp_path, environment=environment)

    results = await agent.execute_tool_calls(
        session,
        [
            unified_llm.ToolCallData(
                id="call-write",
                name="write_file",
                arguments={
                    "path": "nested/output.txt",
                    "content": "delta\n",
                },
            ),
            unified_llm.ToolCallData(
                id="call-read",
                name="read_file",
                arguments={"path": "notes.txt"},
            ),
            unified_llm.ToolCallData(
                id="call-read-many",
                name="read_many_files",
                arguments={"paths": ["notes.txt", "nested/other.txt"]},
            ),
            unified_llm.ToolCallData(
                id="call-missing",
                name="list_dir",
                arguments={"path": "missing-dir"},
            ),
        ],
    )

    assert [result.tool_call_id for result in results] == [
        "call-write",
        "call-read",
        "call-read-many",
        "call-missing",
    ]
    assert all(isinstance(result, unified_llm.ToolResultData) for result in results)

    assert results[0].is_error is False
    assert results[0].content == {
        "path": "nested/output.txt",
        "bytes_written": 6,
    }

    assert results[1].is_error is False
    assert results[1].content == "001 | alpha\n002 | beta"

    assert results[2].is_error is False
    assert results[2].content == {
        "count": 2,
        "files": [
            {
                "path": "notes.txt",
                "content": "001 | alpha\n002 | beta",
            },
            {
                "path": "nested/other.txt",
                "content": "001 | gamma",
            },
        ],
    }

    assert results[3].is_error is True
    assert results[3].content == "File not found: missing-dir"
