from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import unified_llm
import unified_llm.agent as agent
import unified_llm.agent.profiles as profiles
import unified_llm.agent.profiles.anthropic as anthropic_profile_module

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
    def __init__(
        self,
        *,
        working_directory: str = "workspace",
        exec_result: agent.ExecResult | None = None,
    ) -> None:
        self._working_directory = working_directory
        self.exec_result = (
            exec_result
            if exec_result is not None
            else agent.ExecResult(
                stdout="",
                stderr="",
                exit_code=0,
                timed_out=False,
                duration_ms=1,
            )
        )
        self.files: dict[str, str] = {}
        self.exec_calls: list[dict[str, object | None]] = []
        self.grep_calls: list[dict[str, object]] = []
        self.grep_result = ""

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
        return self.exec_result

    def grep(self, pattern: str, path: str | Path, options: agent.GrepOptions) -> str:
        self.grep_calls.append(
            {
                "pattern": pattern,
                "path": path,
                "options": options,
            }
        )
        return self.grep_result

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
        model="claude-sonnet-4-5",
        provider="anthropic",
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


def test_anthropic_profile_factory_is_exported_from_unified_llm_agent() -> None:
    from unified_llm.agent import create_anthropic_profile as imported_create_anthropic_profile

    assert imported_create_anthropic_profile is agent.create_anthropic_profile
    assert imported_create_anthropic_profile is profiles.create_anthropic_profile
    assert imported_create_anthropic_profile is anthropic_profile_module.create_anthropic_profile


def test_anthropic_profile_export_is_shared_between_profile_modules() -> None:
    assert profiles.AnthropicProviderProfile is anthropic_profile_module.AnthropicProviderProfile


def test_anthropic_profile_factory_returns_the_claude_code_style_surface() -> None:
    profile = agent.create_anthropic_profile(model="claude-sonnet-4-5")

    assert isinstance(profile, profiles.AnthropicProviderProfile)
    assert profile.id == "anthropic"
    assert profile.display_name == "Claude Sonnet 4.5"
    assert profile.supports("reasoning") is True
    assert profile.supports("vision") is True
    assert profile.tool_registry.names() == [
        "read_file",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "spawn_agent",
        "send_input",
        "wait",
        "close_agent",
    ]
    assert "apply_patch" not in profile.tool_registry.names()


def test_anthropic_profile_exposes_claude_code_aligned_tool_schemas() -> None:
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")

    assert profile.tool_registry.names() == [
        "read_file",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "spawn_agent",
        "send_input",
        "wait",
        "close_agent",
    ]

    for tool_name in profile.tool_registry.names():
        definition = profile.tool_registry.get(tool_name).definition
        assert definition.parameters["type"] == "object"

    read_file_definition = profile.tool_registry.get("read_file").definition
    assert read_file_definition.parameters["properties"]["file_path"]["type"] == "string"
    assert read_file_definition.parameters["properties"]["offset"]["default"] == 1
    assert read_file_definition.parameters["properties"]["limit"]["default"] == 2000

    write_file_definition = profile.tool_registry.get("write_file").definition
    assert write_file_definition.parameters["properties"]["file_path"]["type"] == "string"
    assert write_file_definition.parameters["properties"]["content"]["type"] == "string"

    edit_file_definition = profile.tool_registry.get("edit_file").definition
    assert edit_file_definition.parameters["properties"]["file_path"]["type"] == "string"
    assert edit_file_definition.parameters["properties"]["old_string"]["type"] == "string"
    assert edit_file_definition.parameters["properties"]["new_string"]["type"] == "string"
    assert "apply_patch" not in profile.tool_registry.names()

    shell_definition = profile.tool_registry.get("shell").definition
    assert shell_definition.parameters["properties"]["timeout_ms"]["default"] == 120_000
    assert shell_definition.parameters["properties"]["command"]["type"] == "string"
    assert shell_definition.parameters["properties"]["description"]["type"] == "string"

    grep_definition = profile.tool_registry.get("grep").definition
    grep_properties = grep_definition.parameters["properties"]
    assert grep_properties["output_mode"]["default"] == "files_with_matches"
    assert set(grep_properties["output_mode"]["enum"]) == {
        "content",
        "files_with_matches",
        "count",
    }
    assert grep_properties["glob"]["type"] == "string"
    assert grep_properties["head_limit"]["default"] == 250
    assert grep_properties["offset"]["default"] == 0


@pytest.mark.asyncio
async def test_anthropic_profile_read_write_edit_and_read_file_interface_use_file_path(
    tmp_path: Path,
) -> None:
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )

    write_result = await _execute_tool(
        session,
        "write_file",
        {
            "file_path": "notes.txt",
            "content": "alpha\nbeta\ngamma\n",
        },
    )
    assert write_result.is_error is False
    assert write_result.content == {
        "path": "notes.txt",
        "bytes_written": 17,
    }

    read_result = await _execute_tool(
        session,
        "read_file",
        {
            "file_path": "notes.txt",
            "offset": 2,
            "limit": 1,
        },
        tool_call_id="read-1",
    )
    assert read_result.is_error is False
    assert read_result.content == "002 | beta"

    edit_result = await _execute_tool(
        session,
        "edit_file",
        {
            "file_path": "notes.txt",
            "old_string": "beta",
            "new_string": "delta",
        },
        tool_call_id="edit-1",
    )
    assert edit_result.is_error is False
    assert edit_result.content["path"] == "notes.txt"
    assert edit_result.content["replacements"] == 1
    assert session.execution_environment.read_file("notes.txt") == "alpha\ndelta\ngamma\n"


@pytest.mark.asyncio
async def test_anthropic_profile_image_reading_is_gated_by_vision_capability(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    (tmp_path / "image.png").write_bytes(PNG_BYTES)

    profile_without_vision = profiles.AnthropicProviderProfile(
        model="claude-sonnet-4-5",
        capabilities={"vision": False},
    )
    session_without_vision = agent.Session(
        profile=profile_without_vision,
        execution_env=environment,
    )

    denied_result = await _execute_tool(
        session_without_vision,
        "read_file",
        {"file_path": "image.png"},
    )
    assert denied_result.is_error is True
    assert denied_result.content == "Binary file not supported: image.png"

    profile_with_vision = profiles.AnthropicProviderProfile(
        model="claude-sonnet-4-5",
        capabilities={"vision": True},
    )
    session_with_vision = agent.Session(
        profile=profile_with_vision,
        execution_env=environment,
    )

    allowed_result = await _execute_tool(
        session_with_vision,
        "read_file",
        {"file_path": "image.png"},
        tool_call_id="read-image",
    )
    assert allowed_result.is_error is False
    assert allowed_result.image_data == PNG_BYTES
    assert allowed_result.image_media_type == "image/png"
    assert "image.png" in allowed_result.content


@pytest.mark.asyncio
async def test_anthropic_profile_latest_wins_custom_overrides_apply_through_the_registry(
    tmp_path: Path,
) -> None:
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
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
        {"file_path": "missing.txt"},
        tool_call_id="call-override",
    )

    assert result.is_error is False
    assert result.content == "custom"
    assert seen_arguments == [{"file_path": "missing.txt"}]


@pytest.mark.asyncio
async def test_anthropic_profile_grep_supports_content_files_and_count_modes(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment()
    environment.grep_result = (
        "workspace/src/app.py:3:alpha:beta\n"
        "workspace/src/app.py:9:omega\n"
        "workspace/nested/tool.py:8:gamma\n"
    )
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
    session = agent.Session(profile=profile, execution_env=environment)

    default_result = await _execute_tool(
        session,
        "grep",
        {
            "pattern": "alpha",
            "glob": "*.py",
            "-i": True,
        },
    )
    assert environment.grep_calls == [
        {
            "pattern": "alpha",
            "path": ".",
            "options": agent.GrepOptions(
                glob_filter="*.py",
                case_insensitive=True,
                max_results=10_000,
            ),
        }
    ]
    assert default_result.is_error is False
    assert default_result.content == [
        "workspace/src/app.py",
        "workspace/nested/tool.py",
    ]

    environment.grep_calls.clear()
    content_result = await _execute_tool(
        session,
        "grep",
        {
            "pattern": "alpha",
            "glob": "*.py",
            "output_mode": "content",
        },
        tool_call_id="grep-content",
    )
    assert environment.grep_calls == [
        {
            "pattern": "alpha",
            "path": ".",
            "options": agent.GrepOptions(
                glob_filter="*.py",
                case_insensitive=False,
                max_results=10_000,
            ),
        }
    ]
    assert content_result.is_error is False
    assert content_result.content == {
        "matches": [
            {
                "path": "workspace/src/app.py",
                "line_number": 3,
                "line": "alpha:beta",
            },
            {
                "path": "workspace/src/app.py",
                "line_number": 9,
                "line": "omega",
            },
            {
                "path": "workspace/nested/tool.py",
                "line_number": 8,
                "line": "gamma",
            },
        ]
    }

    environment.grep_calls.clear()
    count_result = await _execute_tool(
        session,
        "grep",
        {
            "pattern": "alpha",
            "glob": "*.py",
            "output_mode": "count",
        },
        tool_call_id="grep-count",
    )
    assert environment.grep_calls == [
        {
            "pattern": "alpha",
            "path": ".",
            "options": agent.GrepOptions(
                glob_filter="*.py",
                case_insensitive=False,
                max_results=10_000,
            ),
        }
    ]
    assert count_result.is_error is False
    assert count_result.content == {
        "files": [
            {
                "path": "workspace/src/app.py",
                "count": 2,
            },
            {
                "path": "workspace/nested/tool.py",
                "count": 1,
            },
        ]
    }


@pytest.mark.asyncio
async def test_anthropic_shell_accepts_description_and_respects_timeout_bounds() -> None:
    environment = _RecordingToolEnvironment()
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
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
        {"command": "echo hello", "description": "List the workspace"},
    )

    assert environment.exec_calls[-1]["timeout_ms"] == 20_000
    assert result.is_error is False
    assert result.content["stdout"] == ""
    assert result.content["exit_code"] == 0

    environment.exec_calls.clear()
    explicit_result = await _execute_tool(
        session,
        "shell",
        {
            "command": "echo hello",
            "description": "Run a quick command",
            "timeout_ms": 40,
        },
        tool_call_id="shell-2",
    )
    assert environment.exec_calls[-1]["timeout_ms"] == 40
    assert explicit_result.is_error is False


def test_anthropic_profile_provider_options_copy_and_beta_header_mapping() -> None:
    profile = profiles.AnthropicProviderProfile(
        model="claude-sonnet-4-5",
        provider_options_map={"beta_headers": ["prompt-caching-2024-07-31"]},
    )
    environment = agent.LocalExecutionEnvironment(working_dir=Path("."))
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=agent.SessionConfig(),
    )

    request = session.build_request("Session system prompt")

    assert profile.provider_options() == {
        "beta_headers": ["prompt-caching-2024-07-31"],
    }
    returned_options = profile.provider_options()
    returned_options["beta_headers"] = ["mutated"]
    assert profile.provider_options() == {
        "beta_headers": ["prompt-caching-2024-07-31"],
    }
    assert request.provider_options == {
        "anthropic": {
            "beta_headers": ["prompt-caching-2024-07-31"],
        }
    }


def test_anthropic_profile_prompt_surfaces_required_layers_topics_and_edit_file_format(
    tmp_path: Path,
) -> None:
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)

    prompt = profile.build_system_prompt(
        environment,
        {
            "AGENTS.md": "Root guidance",
            "CLAUDE.md": "Claude guidance",
        },
    )

    assert "<provider_base_instructions>" in prompt
    assert "<environment>" in prompt
    assert "<tools>" in prompt
    assert "<project_instructions>" in prompt
    assert "Tool usage:" in prompt
    assert "AGENTS.md" in prompt
    assert "CLAUDE.md" in prompt
    assert "Project instruction conventions:" in prompt
    assert "read before edit" in prompt
    assert "edit_file" in prompt
    assert "write_file" in prompt
    assert "old_string is unique" in prompt
    assert "Coding guidance:" in prompt
    assert "apply_patch" not in prompt


@pytest.mark.asyncio
async def test_anthropic_profile_subagent_tools_execute_through_the_registry(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient(
        [
            _assistant_response("child response 1", "resp-1"),
            _assistant_response("child response 2", "resp-2"),
        ]
    )
    profile = profiles.AnthropicProviderProfile(model="claude-sonnet-4-5")
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
