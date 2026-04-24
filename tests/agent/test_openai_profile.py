from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import agent
import agent.profiles as profiles
import agent.profiles.openai as openai_profile_module
import unified_llm

PNG_BYTES = (
    b"\x89PNG\r\n\x1a\n"
    b"\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01"
    b"\x08\x06\x00\x00\x00\x1f\x15\xc4\x89"
    b"\x00\x00\x00\x0cIDAT\x08\xd7c\xf8\x0f\x00\x01\x01\x01\x00\x18\xdd\x8d\x18"
    b"\x00\x00\x00\x00IEND\xaeB`\x82"
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


class _RecordingToolEnvironment:
    def __init__(self) -> None:
        self.files: dict[str, str] = {}
        self.exec_calls: list[dict[str, object | None]] = []
        self._working_directory = "workspace"

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str | bytes:
        key = str(path)
        if key not in self.files:
            raise FileNotFoundError(key)
        content = self.files[key]
        if offset is None and limit is None:
            return content
        lines = content.splitlines(keepends=True)
        start = 0 if offset is None else offset - 1
        end = None if limit is None else start + limit
        return "".join(lines[start:end])

    def write_file(self, path: str | Path, content: str) -> None:
        self.files[str(path)] = content

    def file_exists(self, path: str | Path) -> bool:
        return str(path) in self.files

    def is_directory(self, path: str | Path) -> bool:
        return False

    def delete_file(self, path: str | Path) -> None:
        self.files.pop(str(path), None)

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        source_key = str(source_path)
        destination_key = str(destination_path)
        self.files[destination_key] = self.files.pop(source_key)

    def list_directory(self, path: str | Path, depth: int) -> list[agent.DirEntry]:
        return []

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: dict[str, str] | None = None,
    ) -> agent.ExecResult:
        self.exec_calls.append(
            {
                "command": command,
                "timeout_ms": timeout_ms,
                "working_dir": working_dir,
                "env_vars": None if env_vars is None else dict(env_vars),
            }
        )
        return agent.ExecResult(
            stdout="ok",
            stderr="",
            exit_code=0,
            timed_out=False,
            duration_ms=1,
        )

    def grep(self, pattern: str, path: str | Path, options: agent.GrepOptions) -> str:
        return ""

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        return []

    def initialize(self) -> None:
        return None

    def cleanup(self) -> None:
        return None

    def working_directory(self) -> str:
        return self._working_directory

    def platform(self) -> str:
        return "linux"

    def os_version(self) -> str:
        return "test-os"


def _assistant_response(text: str, response_id: str) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model="gpt-5.2",
        provider="openai",
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason.STOP,
    )


async def _execute_tool(
    session: agent.Session,
    tool_name: str,
    arguments: dict[str, object],
    *,
    tool_call_id: str = "call-1",
) -> unified_llm.ToolResultData:
    return await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id=tool_call_id,
            name=tool_name,
            arguments=arguments,
        ),
    )


def test_openai_profile_factory_is_exported_from_unified_llm_agent() -> None:
    from agent import create_openai_profile as imported_create_openai_profile

    assert imported_create_openai_profile is agent.create_openai_profile
    assert imported_create_openai_profile is profiles.create_openai_profile
    assert imported_create_openai_profile is openai_profile_module.create_openai_profile


def test_openai_profile_factory_returns_the_openai_profile_surface() -> None:
    profile = agent.create_openai_profile(model="gpt-5.2")

    assert isinstance(profile, profiles.OpenAIProviderProfile)
    assert profile.id == "openai"
    assert profile.tool_registry.names() == [
        "read_file",
        "apply_patch",
        "write_file",
        "shell",
        "grep",
        "glob",
        "spawn_agent",
        "send_input",
        "wait",
        "close_agent",
    ]


def test_openai_profile_export_is_shared_between_profile_modules() -> None:
    assert profiles.OpenAIProviderProfile is openai_profile_module.OpenAIProviderProfile


def test_openai_profile_exposes_codex_style_tool_registry_and_object_root_schemas() -> None:
    profile = profiles.OpenAIProviderProfile(model="gpt-5.2")

    assert profile.id == "openai"
    assert profile.display_name == "GPT-5.2"
    assert profile.supports("reasoning") is True
    assert profile.supports("vision") is True
    assert profile.tool_registry.names() == [
        "read_file",
        "apply_patch",
        "write_file",
        "shell",
        "grep",
        "glob",
        "spawn_agent",
        "send_input",
        "wait",
        "close_agent",
    ]
    assert "edit_file" not in profile.tool_registry.names()
    for tool_name in profile.tool_registry.names():
        definition = profile.tool_registry.get(tool_name).definition
        assert definition.parameters["type"] == "object"

    shell_definition = profile.tool_registry.get("shell").definition
    assert shell_definition.parameters["properties"]["timeout_ms"]["default"] == 10_000
    assert shell_definition.parameters["properties"]["command"]["type"] == "string"
    assert profile.tool_registry.get("apply_patch").definition.parameters["properties"][
        "patch"
    ]["type"] == "string"


@pytest.mark.asyncio
async def test_openai_profile_filters_project_docs_and_builds_requests(
    tmp_path: Path,
) -> None:
    (tmp_path / "AGENTS.md").write_text("Root guidance")
    (tmp_path / ".codex/instructions.md").parent.mkdir(parents=True, exist_ok=True)
    (tmp_path / ".codex/instructions.md").write_text("OpenAI guidance")
    (tmp_path / "CLAUDE.md").write_text("Anthropic guidance")
    (tmp_path / "GEMINI.md").write_text("Gemini guidance")

    profile = profiles.OpenAIProviderProfile(model="gpt-5.2")
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    client = _BlockingCompleteClient([_assistant_response("done", "resp-1")])
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
    )

    processing_task = asyncio.create_task(session.process_input("Check the workspace"))
    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    request = client.requests[0]
    system_prompt = request.messages[0].text

    assert request.provider == "openai"
    assert [tool.name for tool in request.tools] == profile.tool_registry.names()
    assert request.tool_choice is not None
    assert request.tool_choice.mode == "auto"
    assert request.messages[0].role == unified_llm.Role.SYSTEM
    assert "AGENTS.md" in system_prompt
    assert ".codex/instructions.md" in system_prompt
    assert "CLAUDE.md" not in system_prompt
    assert "GEMINI.md" not in system_prompt

    client.released[0].set()
    await processing_task
    assert session.state == agent.SessionState.IDLE
    await session.close()
    assert session.state == agent.SessionState.CLOSED


@pytest.mark.asyncio
async def test_openai_profile_latest_wins_custom_overrides_apply_through_the_registry(
    tmp_path: Path,
) -> None:
    profile = profiles.OpenAIProviderProfile(model="gpt-5.2")
    custom_definition = agent.ToolDefinition(
        name="read_file",
        description="Custom read",
        parameters={"type": "object"},
    )
    seen_arguments: list[dict[str, object]] = []

    profile.tool_registry.register(
        custom_definition,
        executor=lambda arguments, execution_environment: (
            seen_arguments.append(dict(arguments)) or "custom"
        ),
    )

    assert profile.tool_registry.get("read_file").definition is custom_definition

    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )
    result = await _execute_tool(
        session,
        "read_file",
        {"path": "missing.txt"},
        tool_call_id="call-override",
    )

    assert result.is_error is False
    assert result.content == "custom"
    assert seen_arguments == [{"path": "missing.txt"}]


@pytest.mark.parametrize(
    ("capabilities", "expected_success"),
    [
        ({"vision": False}, False),
        ({"vision": True}, True),
    ],
)
@pytest.mark.asyncio
async def test_openai_profile_image_reading_is_gated_by_vision_capability(
    tmp_path: Path,
    capabilities: dict[str, bool],
    expected_success: bool,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    (tmp_path / "image.png").write_bytes(PNG_BYTES)
    profile = profiles.OpenAIProviderProfile(model="gpt-5.2", capabilities=capabilities)
    session = agent.Session(profile=profile, execution_env=environment)

    result = await _execute_tool(
        session,
        "read_file",
        {"path": "image.png"},
    )

    assert result.is_error is (not expected_success)
    if expected_success:
        assert result.image_data == PNG_BYTES
        assert result.image_media_type == "image/png"
        assert "image.png" in result.content
    else:
        assert result.content == "Binary file not supported: image.png"


@pytest.mark.asyncio
async def test_openai_shell_uses_a_10_second_default_and_respects_bounds() -> None:
    environment = _RecordingToolEnvironment()
    profile = profiles.OpenAIProviderProfile(model="gpt-5.2")
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=agent.SessionConfig(
            default_command_timeout_ms=75,
            max_command_timeout_ms=20_000,
        ),
    )

    result = await _execute_tool(
        session,
        "shell",
        {"command": "echo hello"},
    )

    assert environment.exec_calls[-1]["timeout_ms"] == 10_000
    assert result.is_error is False
    assert result.content["stdout"] == "ok"

    environment.exec_calls.clear()
    session.config.max_command_timeout_ms = 50
    capped_result = await _execute_tool(
        session,
        "shell",
        {"command": "echo hello"},
        tool_call_id="call-2",
    )
    assert environment.exec_calls[-1]["timeout_ms"] == 50
    assert capped_result.is_error is False

    environment.exec_calls.clear()
    explicit_result = await _execute_tool(
        session,
        "shell",
        {"command": "echo hello", "timeout_ms": 40},
        tool_call_id="call-3",
    )
    assert environment.exec_calls[-1]["timeout_ms"] == 40
    assert explicit_result.is_error is False


def test_openai_profile_provider_options_copy_and_reasoning_effort_mapping() -> None:
    profile = profiles.OpenAIProviderProfile(
        model="gpt-5.2",
        provider_options_map={"temperature": 0.2},
    )
    environment = agent.LocalExecutionEnvironment(working_dir=Path("."))
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=agent.SessionConfig(reasoning_effort="high"),
    )

    request = session.build_request("Session system prompt")

    assert profile.provider_options() == {"temperature": 0.2}
    assert request.reasoning_effort == "high"
    assert request.provider == "openai"
    assert request.provider_options == {
        "openai": {
            "temperature": 0.2,
            "reasoning": {"effort": "high"},
        }
    }


@pytest.mark.asyncio
async def test_openai_profile_subagent_tools_execute_through_the_registry(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient(
        [
            _assistant_response("child response 1", "resp-1"),
            _assistant_response("child response 2", "resp-2"),
        ]
    )
    profile = profiles.OpenAIProviderProfile(model="gpt-5.2")
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
        llm_client=client,
        config=agent.SessionConfig(max_subagent_depth=1),
    )

    spawn_result = await _execute_tool(
        session,
        "spawn_agent",
        {"task": "Investigate the repository"},
        tool_call_id="spawn-1",
    )

    assert spawn_result.is_error is False
    assert spawn_result.content["status"] == "running"
    agent_id = spawn_result.content["agent_id"]

    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    send_result = await _execute_tool(
        session,
        "send_input",
        {
            "agent_id": agent_id,
            "message": "Please continue",
        },
        tool_call_id="send-1",
    )

    assert send_result.is_error is False
    assert send_result.content["status"] == "running"

    client.released[0].set()
    await asyncio.wait_for(client.started[1].wait(), timeout=1)

    wait_task = asyncio.create_task(
        _execute_tool(
            session,
            "wait",
            {"agent_id": agent_id},
            tool_call_id="wait-1",
        )
    )
    client.released[1].set()
    wait_result = await asyncio.wait_for(wait_task, timeout=1)

    assert wait_result.is_error is False
    assert wait_result.content["agent_id"] == agent_id
    assert wait_result.content["status"] == "completed"
    assert wait_result.content["success"] is True
    assert wait_result.content["output"] == "child response 2"

    close_result = await _execute_tool(
        session,
        "close_agent",
        {"agent_id": agent_id},
        tool_call_id="close-1",
    )
    assert close_result.is_error is False
    assert close_result.content["status"] == "completed"
