from __future__ import annotations

import asyncio
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

import agent
import agent.subagents as subagents
import unified_llm


def _shell_command(*args: str) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(list(args))
    return shlex.join(args)


def _python_command(code: str) -> str:
    return _shell_command(sys.executable, "-c", code)


def _make_parent_session(
    tmp_path: Path,
    *,
    model: str = "parent-model",
    max_turns: int = 0,
    max_subagent_depth: int = 1,
) -> agent.Session:
    profile = agent.ProviderProfile(
        id="provider-id",
        model=model,
        capabilities={"tool_calls": True},
        provider_options_map={"temperature": 0.2},
        display_name="Parent",
    )
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path / "workspace")
    config = agent.SessionConfig(
        max_turns=max_turns,
        max_subagent_depth=max_subagent_depth,
    )
    return agent.Session(
        profile=profile,
        execution_env=environment,
        config=config,
    )


def _make_subagent_session(
    tmp_path: Path,
    *,
    client: object | None = None,
    max_subagent_depth: int = 1,
) -> agent.Session:
    profile = agent.ProviderProfile(
        id="provider-id",
        model="parent-model",
        capabilities={"tool_calls": True},
        provider_options_map={"temperature": 0.2},
        display_name="Parent",
    )
    profile.tool_registry = subagents.register_subagent_tools(provider_profile=profile)
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path / "workspace")
    config = agent.SessionConfig(
        max_subagent_depth=max_subagent_depth,
    )
    return agent.Session(
        profile=profile,
        execution_env=environment,
        config=config,
        llm_client=client,
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


def _assistant_response(text: str, response_id: str) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="parent-model",
        provider="provider-id",
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason.STOP,
    )


def test_create_child_session_keeps_history_independent_and_shares_environment(
    tmp_path: Path,
) -> None:
    parent = _make_parent_session(tmp_path)
    parent.history.append(agent.UserTurn(content="parent turn"))

    handle = subagents.create_child_session(parent)
    child = handle.session

    assert child is not None
    assert handle.status is agent.SubAgentStatus.PENDING
    assert handle.session_id == child.id
    assert handle.working_directory == Path(child.execution_environment.working_directory())
    assert child.execution_environment is parent.execution_environment
    assert child.history == []
    assert parent.active_subagents[handle.id] is handle

    child.history.append(agent.UserTurn(content="child turn"))
    child.execution_environment.write_file("shared.txt", "child data")

    assert [turn.text for turn in parent.history] == ["parent turn"]
    assert parent.execution_environment.read_file("shared.txt") == "child data"


def test_create_child_session_scopes_working_directory_and_blocks_escape(
    tmp_path: Path,
) -> None:
    parent = _make_parent_session(tmp_path)

    handle = subagents.create_child_session(parent, working_dir="child")
    child = handle.session

    assert child is not None
    assert Path(child.execution_environment.working_directory()) == (
        tmp_path / "workspace" / "child"
    )

    child.execution_environment.write_file("note.txt", "hello")

    assert parent.execution_environment.file_exists("child/note.txt") is True
    assert parent.execution_environment.read_file("child/note.txt") == "hello"

    with pytest.raises(ValueError, match="working_dir"):
        subagents.create_child_session(parent, working_dir="../escape")


@pytest.mark.parametrize(
    "invoke",
    [
        lambda env: env.read_file("../escaped.txt"),
        lambda env: env.write_file("../escaped.txt", "nope"),
        lambda env: env.file_exists("../escaped.txt"),
        lambda env: env.is_directory("../escaped.txt"),
        lambda env: env.delete_file("../escaped.txt"),
        lambda env: env.rename_file("../escaped.txt", "../other.txt"),
        lambda env: env.list_directory("..", depth=0),
        lambda env: env.grep("needle", "..", agent.GrepOptions()),
        lambda env: env.glob("*.txt", ".."),
        lambda env: env.exec_command(
            _python_command("import os; print(os.getcwd())"),
            working_dir="../escape",
            timeout_ms=1000,
        ),
    ],
    ids=[
        "read_file",
        "write_file",
        "file_exists",
        "is_directory",
        "delete_file",
        "rename_file",
        "list_directory",
        "grep",
        "glob",
        "exec_command",
    ],
)
def test_create_child_session_scoped_environment_blocks_post_spawn_escape_attempts(
    tmp_path: Path,
    invoke,
) -> None:
    parent = _make_parent_session(tmp_path)
    handle = subagents.create_child_session(parent, working_dir="child")
    child = handle.session

    assert child is not None
    assert Path(child.execution_environment.working_directory()) == (
        tmp_path / "workspace" / "child"
    )

    with pytest.raises(PermissionError, match="escapes scoped working directory"):
        invoke(child.execution_environment)


def test_create_child_session_clones_provider_profile_and_overrides_model(
    tmp_path: Path,
) -> None:
    parent = _make_parent_session(tmp_path)
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )
    parent.provider_profile.tool_registry.register(
        tool_definition,
        executor=lambda arguments, execution_environment: "ok",
    )
    parent.provider_profile.capabilities["custom"] = True
    parent.provider_profile.provider_options_map["top_p"] = 0.9

    handle = subagents.create_child_session(parent, model="child-model")
    child = handle.session

    assert child is not None
    assert child.provider_profile is handle.provider_profile
    assert child.provider_profile is not parent.provider_profile
    assert child.provider_profile.tool_registry is not parent.provider_profile.tool_registry
    assert child.provider_profile.model == "child-model"
    assert child.provider_profile.id == parent.provider_profile.id
    assert child.provider_profile.tools() == parent.provider_profile.tools()
    assert child.provider_profile.capabilities == parent.provider_profile.capabilities
    assert child.provider_profile.provider_options() == parent.provider_profile.provider_options()
    assert child.provider_profile.supports("tool_calls") is True

    child.provider_profile.tool_registry.unregister("lookup")
    assert parent.provider_profile.tool_registry.get("lookup") is not None
    child.provider_profile.capabilities["child-only"] = True
    assert "child-only" not in parent.provider_profile.capabilities


def test_create_child_session_overrides_child_turn_limit_independently(
    tmp_path: Path,
) -> None:
    parent = _make_parent_session(tmp_path, max_turns=11, max_subagent_depth=2)

    default_handle = subagents.create_child_session(parent)
    default_child = default_handle.session
    assert default_child is not None
    assert default_child.config.max_turns == 0
    assert default_child.config.max_subagent_depth == 1

    overridden_handle = subagents.create_child_session(parent, max_turns=4)
    overridden_child = overridden_handle.session
    assert overridden_child is not None
    assert overridden_child.config.max_turns == 4
    assert overridden_child.config.max_subagent_depth == 1


def test_create_child_session_blocks_recursive_spawning_at_depth_limit(
    tmp_path: Path,
) -> None:
    parent = _make_parent_session(tmp_path, max_subagent_depth=1)

    child_handle = subagents.create_child_session(parent)
    child = child_handle.session
    assert child is not None
    assert child.config.max_subagent_depth == 0

    with pytest.raises(agent.SubAgentLimitError, match="max_subagent_depth"):
        subagents.create_child_session(child)


@pytest.mark.asyncio
async def test_subagent_tools_spawn_send_wait_and_close_through_the_tool_registry(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient(
        [
            _assistant_response("child response 1", "resp-1"),
            _assistant_response("child response 2", "resp-2"),
        ]
    )
    session = _make_subagent_session(tmp_path, client=client)

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )

    assert spawn_result.is_error is False
    assert spawn_result.content["status"] == "running"
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))
    assert handle.status == agent.SubAgentStatus.RUNNING

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
    client.released[1].set()
    wait_result = await asyncio.wait_for(wait_task, timeout=1)

    assert wait_result.is_error is False
    assert wait_result.content["agent_id"] == agent_id
    assert wait_result.content["status"] == "completed"
    assert wait_result.content["success"] is True
    assert wait_result.content["output"] == "child response 2"
    assert wait_result.content["turns_used"] == 4
    assert handle.result is not None
    assert handle.result.status == agent.SubAgentStatus.COMPLETED

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

    completed_error = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="send-2",
            name="send_input",
            arguments={
                "agent_id": agent_id,
                "message": "Too late",
            },
        ),
    )
    assert completed_error.is_error is True
    assert "completed" in completed_error.content

    await session.close()


@pytest.mark.asyncio
async def test_subagent_spawn_reports_startup_failure_recoverably(
    tmp_path: Path,
) -> None:
    session = _make_subagent_session(tmp_path, max_subagent_depth=0)

    result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-failure",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )

    assert result.is_error is True
    assert "max_subagent_depth" in result.content
    assert session.active_subagents == {}
    await session.close()


@pytest.mark.asyncio
async def test_subagent_wait_reports_failed_child_tasks_and_blocks_follow_up(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient(
        [_assistant_response("child response", "resp-1")],
        errors=[RuntimeError("boom")],
    )
    session = _make_subagent_session(tmp_path, client=client)

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-failure-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )
    agent_id = spawn_result.content["agent_id"]

    await asyncio.wait_for(client.started[0].wait(), timeout=1)
    client.released[0].set()

    wait_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="wait-failure-1",
            name="wait",
            arguments={"agent_id": agent_id},
        ),
    )

    assert wait_result.is_error is False
    assert wait_result.content["status"] == "failed"
    assert wait_result.content["success"] is False
    assert wait_result.content["output"] is None
    assert "boom" in wait_result.content["error"]

    follow_up_error = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="send-failure-1",
            name="send_input",
            arguments={
                "agent_id": agent_id,
                "message": "Still there?",
            },
        ),
    )

    assert follow_up_error.is_error is True
    assert "failed" in follow_up_error.content

    await session.close()


@pytest.mark.asyncio
async def test_subagent_close_agent_closes_running_task_and_cleanup_removes_handles(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient([_assistant_response("child response", "resp-1")])
    session = _make_subagent_session(tmp_path, client=client)

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-close-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))

    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    close_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="close-running-1",
            name="close_agent",
            arguments={"agent_id": agent_id},
        ),
    )

    assert close_result.is_error is False
    assert close_result.content["status"] == "closed"
    assert handle.status == agent.SubAgentStatus.CLOSED
    assert handle.task is not None
    assert handle.task.done() is True

    closed_error = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="send-closed-1",
            name="send_input",
            arguments={
                "agent_id": agent_id,
                "message": "Hello?",
            },
        ),
    )

    assert closed_error.is_error is True
    assert "closed" in closed_error.content

    await session.close()


@pytest.mark.asyncio
async def test_session_close_cleans_up_running_child_tasks(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient([_assistant_response("child response", "resp-1")])
    session = _make_subagent_session(tmp_path, client=client)

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-cleanup-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))

    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    await session.close()

    assert session.state == agent.SessionState.CLOSED
    assert session.active_subagents == {}
    assert handle.status == agent.SubAgentStatus.CLOSED
    assert handle.task is not None
    assert handle.task.done() is True
    assert handle.result is not None
    assert handle.result.status == agent.SubAgentStatus.CLOSED
    assert agent_id
