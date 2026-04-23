from __future__ import annotations

import asyncio
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

import unified_llm
import unified_llm.agent as agent
import unified_llm.agent.builtin_tools as builtin_tools
import unified_llm.agent.subagents as subagents


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def _shell_command(*args: str) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(list(args))
    return shlex.join(args)


def _python_command(code: str) -> str:
    return _shell_command(sys.executable, "-c", code)


async def _wait_for_path(path: Path, *, timeout: float = 3.0) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    while not path.exists():
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise AssertionError(f"timed out waiting for {path}")
        await asyncio.sleep(min(0.05, remaining))


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


class _RecordingEnvironment:
    def __init__(self, working_directory: Path) -> None:
        self._working_directory = Path(working_directory)
        self.cleanup_calls = 0
        self.child_environments: list[_RecordingEnvironment] = []

    def working_directory(self) -> str:
        return str(self._working_directory)

    def cleanup(self) -> None:
        self.cleanup_calls += 1

    def with_working_directory(
        self,
        working_dir: str | Path | None = None,
    ) -> _RecordingEnvironment:
        configured = self._working_directory if working_dir is None else Path(working_dir)
        child_environment = _RecordingEnvironment(configured)
        self.child_environments.append(child_environment)
        return child_environment


class _PausingCompleteClient:
    def __init__(self, response: unified_llm.Response) -> None:
        self.requests: list[unified_llm.Request] = []
        self._response = response
        self.complete_started = asyncio.Event()
        self.release_complete = asyncio.Event()
        self.cancelled = asyncio.Event()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        self.complete_started.set()
        try:
            await self.release_complete.wait()
        except asyncio.CancelledError:
            self.cancelled.set()
            raise
        return self._response


class _ToolCallClient:
    def __init__(self, response: unified_llm.Response) -> None:
        self.requests: list[unified_llm.Request] = []
        self._response = response

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        return self._response


def _assistant_response(text: str, response_id: str) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="fake-model",
        provider="fake-provider",
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason.STOP,
    )


def _tool_call_response(command: str) -> unified_llm.Response:
    tool_call = unified_llm.ToolCallData(
        id="call-1",
        name="shell",
        arguments={"command": command},
    )
    return unified_llm.Response(
        id="resp-1",
        model="fake-model",
        provider="fake-provider",
        message=unified_llm.Message.assistant(
            [
                unified_llm.ContentPart(
                    kind=unified_llm.ContentKind.TOOL_CALL,
                    tool_call=tool_call,
                )
            ]
        ),
        finish_reason=unified_llm.FinishReason(
            reason=unified_llm.FinishReason.TOOL_CALLS,
        ),
    )


@pytest.mark.asyncio
async def test_session_close_cancels_in_flight_model_calls_and_cleans_up_environment(
    tmp_path: Path,
) -> None:
    environment = _RecordingEnvironment(tmp_path)
    client = _PausingCompleteClient(_assistant_response("done", "resp-1"))
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        execution_env=environment,
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    processing_task = asyncio.create_task(session.process_input("Question"))
    await asyncio.wait_for(client.complete_started.wait(), timeout=1)

    await session.close()

    with pytest.raises(asyncio.CancelledError):
        await processing_task

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    assert client.cancelled.is_set() is True
    assert environment.cleanup_calls == 1
    assert session.state == agent.SessionState.CLOSED
    assert session.history[0].text == "Question"


@pytest.mark.asyncio
async def test_session_close_terminates_an_in_flight_shell_command(
    tmp_path: Path,
) -> None:
    if os.name == "nt" or not hasattr(os, "killpg"):
        pytest.skip("POSIX signal handling is not available on Windows")

    started_marker = tmp_path / "shell-started.txt"
    terminated_marker = tmp_path / "shell-terminated.txt"
    shell_script = (
        "import pathlib, signal, sys, time\n"
        f"started = pathlib.Path({str(started_marker)!r})\n"
        f"terminated = pathlib.Path({str(terminated_marker)!r})\n"
        "started.write_text('started', encoding='utf-8')\n"
        "def handler(*_):\n"
        "    terminated.write_text('terminated', encoding='utf-8')\n"
        "    sys.exit(0)\n"
        "signal.signal(signal.SIGTERM, handler)\n"
        "time.sleep(30)\n"
    )
    command = _python_command(shell_script)
    profile = _PromptProfile(id="fake-provider", model="fake-model")
    profile.tool_registry = builtin_tools.register_builtin_tools(provider_profile=profile)
    client = _ToolCallClient(_tool_call_response(command))
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    processing_task = asyncio.create_task(session.process_input("Run the command"))

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Run the command"}
    assistant_text_start = await _next_event(stream)
    assert assistant_text_start.kind == agent.EventKind.ASSISTANT_TEXT_START
    assistant_text_delta = await _next_event(stream)
    assert assistant_text_delta.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assistant_text_end = await _next_event(stream)
    assert assistant_text_end.kind == agent.EventKind.ASSISTANT_TEXT_END
    tool_start_event = await _next_event(stream)
    assert tool_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert tool_start_event.data == {"tool_call_id": "call-1", "tool_name": "shell"}

    await _wait_for_path(started_marker)
    await session.close()
    await _wait_for_path(terminated_marker)

    assert terminated_marker.read_text(encoding="utf-8") == "terminated"
    with pytest.raises(asyncio.CancelledError):
        await processing_task

    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}
    assert session.state == agent.SessionState.CLOSED
    assert len(client.requests) == 1


@pytest.mark.asyncio
async def test_session_close_closes_active_subagents_through_the_normal_cleanup_path(
    tmp_path: Path,
) -> None:
    environment = _RecordingEnvironment(tmp_path)
    client = _PausingCompleteClient(_assistant_response("child response", "resp-1"))
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        execution_env=environment,
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    spawn_result = subagents.spawn_agent(
        {"task": "Investigate the repository", "working_dir": "child"},
        session.execution_environment,
        session=session,
    )
    assert spawn_result.is_error is False
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))
    child_session = handle.session
    assert child_session is not None

    await asyncio.wait_for(client.complete_started.wait(), timeout=1)

    await session.close()

    assert client.cancelled.is_set() is True
    assert environment.cleanup_calls == 1
    assert session.state == agent.SessionState.CLOSED
    assert session.active_subagents == {}
    assert handle.status == agent.SubAgentStatus.CLOSED
    assert handle.task is not None
    assert handle.task.done() is True
    assert handle.result is not None
    assert handle.result.status == agent.SubAgentStatus.CLOSED
    assert agent_id == str(handle.id)
    assert len(environment.child_environments) == 1
    assert environment.child_environments[0].cleanup_calls == 1
    assert environment.cleanup_calls == 1

    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}
