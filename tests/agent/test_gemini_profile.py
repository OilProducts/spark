from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import agent
import agent.profiles as profiles
import agent.profiles.gemini as gemini_profile_module
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
        self.list_directory_calls: list[dict[str, object]] = []
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
        self.list_directory_calls.append({"path": str(path), "depth": depth})
        return [agent.DirEntry(name="child.txt", is_dir=False, size=3)]

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
        model="gemini-3.1-pro-preview",
        provider="gemini",
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


def test_gemini_profile_factory_is_exported_from_unified_llm_agent() -> None:
    from agent import create_gemini_profile as imported_create_gemini_profile

    assert imported_create_gemini_profile is agent.create_gemini_profile
    assert imported_create_gemini_profile is profiles.create_gemini_profile
    assert imported_create_gemini_profile is gemini_profile_module.create_gemini_profile


def test_gemini_profile_export_is_shared_between_profile_modules() -> None:
    assert profiles.GeminiProviderProfile is gemini_profile_module.GeminiProviderProfile


def test_gemini_profile_factory_returns_the_gemini_cli_style_surface() -> None:
    profile = agent.create_gemini_profile(model="gemini-3.1-pro-preview")

    assert isinstance(profile, profiles.GeminiProviderProfile)
    assert profile.id == "gemini"
    assert profile.display_name == "Gemini 3.1 Pro Preview"
    assert profile.supports("reasoning") is True
    assert profile.supports("vision") is True
    assert profile.shell_timeout_ms == 10_000
    assert profile.tool_registry.names() == [
        "read_file",
        "read_many_files",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "list_dir",
        "spawn_agent",
        "send_input",
        "wait",
        "close_agent",
    ]
    assert "web_search" not in profile.tool_registry.names()
    assert "web_fetch" not in profile.tool_registry.names()


def test_gemini_profile_exposes_cli_aligned_tool_schemas() -> None:
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")

    assert profile.tool_registry.names() == [
        "read_file",
        "read_many_files",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "list_dir",
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

    read_many_definition = profile.tool_registry.get("read_many_files").definition
    assert read_many_definition.parameters["properties"]["paths"]["type"] == "array"
    assert read_many_definition.parameters["properties"]["paths"]["minItems"] == 1

    write_file_definition = profile.tool_registry.get("write_file").definition
    assert write_file_definition.parameters["properties"]["file_path"]["type"] == "string"
    assert write_file_definition.parameters["properties"]["content"]["type"] == "string"

    edit_file_definition = profile.tool_registry.get("edit_file").definition
    edit_properties = edit_file_definition.parameters["properties"]
    assert edit_properties["file_path"]["type"] == "string"
    assert edit_properties["instruction"]["type"] == "string"
    assert edit_properties["old_string"]["type"] == "string"
    assert edit_properties["new_string"]["type"] == "string"
    assert edit_properties["allow_multiple"]["default"] is False
    assert "replace_all" not in edit_properties

    shell_definition = profile.tool_registry.get("shell").definition
    assert shell_definition.parameters["properties"]["timeout_ms"]["default"] == 10_000
    assert shell_definition.parameters["properties"]["command"]["type"] == "string"

    list_dir_definition = profile.tool_registry.get("list_dir").definition
    assert list_dir_definition.parameters["properties"]["path"]["type"] == "string"
    assert list_dir_definition.parameters["properties"]["depth"]["default"] == 0


@pytest.mark.parametrize(
    ("enable_web_search", "enable_web_fetch", "expected_names"),
    [
        (
            False,
            False,
            [
                "read_file",
                "read_many_files",
                "write_file",
                "edit_file",
                "shell",
                "grep",
                "glob",
                "list_dir",
                "spawn_agent",
                "send_input",
                "wait",
                "close_agent",
            ],
        ),
        (
            True,
            False,
            [
                "read_file",
                "read_many_files",
                "write_file",
                "edit_file",
                "shell",
                "grep",
                "glob",
                "list_dir",
                "web_search",
                "spawn_agent",
                "send_input",
                "wait",
                "close_agent",
            ],
        ),
        (
            False,
            True,
            [
                "read_file",
                "read_many_files",
                "write_file",
                "edit_file",
                "shell",
                "grep",
                "glob",
                "list_dir",
                "web_fetch",
                "spawn_agent",
                "send_input",
                "wait",
                "close_agent",
            ],
        ),
        (
            True,
            True,
            [
                "read_file",
                "read_many_files",
                "write_file",
                "edit_file",
                "shell",
                "grep",
                "glob",
                "list_dir",
                "web_search",
                "web_fetch",
                "spawn_agent",
                "send_input",
                "wait",
                "close_agent",
            ],
        ),
    ],
)
def test_gemini_optional_web_tools_are_gated_by_profile_configuration(
    enable_web_search: bool,
    enable_web_fetch: bool,
    expected_names: list[str],
) -> None:
    profile = profiles.GeminiProviderProfile(
        model="gemini-3.1-pro-preview",
        enable_web_search=enable_web_search,
        enable_web_fetch=enable_web_fetch,
    )

    assert profile.tool_registry.names() == expected_names
    assert ("web_search" in expected_names) is enable_web_search
    assert ("web_fetch" in expected_names) is enable_web_fetch


@pytest.mark.parametrize(
    ("capabilities", "expected_success"),
    [
        ({"vision": False}, False),
        ({"vision": True}, True),
    ],
)
@pytest.mark.asyncio
async def test_gemini_profile_image_reading_is_gated_by_vision_capability(
    tmp_path: Path,
    capabilities: dict[str, bool],
    expected_success: bool,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    (tmp_path / "image.png").write_bytes(PNG_BYTES)
    profile = profiles.GeminiProviderProfile(
        model="gemini-3.1-pro-preview",
        capabilities=capabilities,
    )
    session = agent.Session(profile=profile, execution_env=environment)

    result = await _execute_tool(
        session,
        "read_file",
        {"file_path": "image.png"},
    )

    assert result.is_error is (not expected_success)
    if expected_success:
        assert result.image_data == PNG_BYTES
        assert result.image_media_type == "image/png"
        assert "image.png" in result.content
    else:
        assert result.content == "Binary file not supported: image.png"


@pytest.mark.asyncio
async def test_gemini_profile_read_write_edit_and_read_many_files_use_gemini_style_arguments(
    tmp_path: Path,
) -> None:
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
    )

    write_result = await _execute_tool(
        session,
        "write_file",
        {
            "file_path": "notes.txt",
            "content": "alpha\nbeta\nalpha\n",
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
            "instruction": "Replace alpha with gamma",
            "old_string": "alpha",
            "new_string": "gamma",
            "allow_multiple": True,
        },
        tool_call_id="edit-1",
    )
    assert edit_result.is_error is False
    assert edit_result.content["path"] == "notes.txt"
    assert edit_result.content["replacements"] == 2
    assert session.execution_environment.read_file("notes.txt") == "gamma\nbeta\ngamma\n"

    other_write_result = await _execute_tool(
        session,
        "write_file",
        {
            "file_path": "other.txt",
            "content": "delta\n",
        },
        tool_call_id="write-2",
    )
    assert other_write_result.is_error is False

    many_result = await _execute_tool(
        session,
        "read_many_files",
        {
            "paths": ["notes.txt", "other.txt"],
        },
        tool_call_id="many-1",
    )
    assert many_result.is_error is False
    assert many_result.content["count"] == 2
    assert [record["path"] for record in many_result.content["files"]] == [
        "notes.txt",
        "other.txt",
    ]


@pytest.mark.asyncio
async def test_gemini_profile_list_dir_uses_depth_through_the_public_tool_interface() -> None:
    environment = _RecordingToolEnvironment()
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
    session = agent.Session(profile=profile, execution_env=environment)

    result = await _execute_tool(
        session,
        "list_dir",
        {
            "path": "src",
            "depth": 2,
        },
        tool_call_id="list-1",
    )

    assert environment.list_directory_calls == [{"path": "src", "depth": 2}]
    assert result.is_error is False
    assert result.content == {
        "path": "src",
        "depth": 2,
        "count": 1,
        "entries": [
            {
                "name": "child.txt",
                "is_dir": False,
                "size": 3,
            }
        ],
    }


@pytest.mark.asyncio
async def test_gemini_profile_shell_uses_a_10_second_default_and_respects_bounds() -> None:
    environment = _RecordingToolEnvironment()
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
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


def test_gemini_profile_provider_options_copy_and_request_mapping(tmp_path: Path) -> None:
    profile = profiles.GeminiProviderProfile(
        model="gemini-3.1-pro-preview",
        provider_options_map={
            "safety_settings": {
                "category": "HARM_CATEGORY_HATE_SPEECH",
                "threshold": "BLOCK_ONLY_HIGH",
            },
            "grounding": {
                "search": {"mode": "dynamic"},
            },
            "thinking": {
                "thinkingConfig": {"thinkingBudget": 1024},
            },
            "request_options": {
                "cachedContent": "projects/example/cachedContents/abc123",
            },
        },
    )
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=agent.SessionConfig(reasoning_effort="medium"),
    )

    request = session.build_request("Session system prompt")

    assert profile.provider_options() == {
        "safety_settings": {
            "category": "HARM_CATEGORY_HATE_SPEECH",
            "threshold": "BLOCK_ONLY_HIGH",
        },
        "grounding": {
            "search": {"mode": "dynamic"},
        },
        "thinking": {
            "thinkingConfig": {"thinkingBudget": 1024},
        },
        "request_options": {
            "cachedContent": "projects/example/cachedContents/abc123",
        },
    }
    assert request.provider == "gemini"
    assert request.reasoning_effort == "medium"
    assert request.provider_options == {
        "gemini": {
            "safety_settings": {
                "category": "HARM_CATEGORY_HATE_SPEECH",
                "threshold": "BLOCK_ONLY_HIGH",
            },
            "grounding": {
                "search": {"mode": "dynamic"},
            },
            "thinking": {
                "thinkingConfig": {"thinkingBudget": 1024},
            },
            "request_options": {
                "cachedContent": "projects/example/cachedContents/abc123",
            },
        }
    }


@pytest.mark.asyncio
async def test_gemini_profile_filters_project_docs_and_builds_requests(
    tmp_path: Path,
) -> None:
    (tmp_path / "AGENTS.md").write_text("Root guidance")
    (tmp_path / "GEMINI.md").write_text("Gemini guidance")
    (tmp_path / ".codex/instructions.md").parent.mkdir(parents=True, exist_ok=True)
    (tmp_path / ".codex/instructions.md").write_text("OpenAI guidance")
    (tmp_path / "CLAUDE.md").write_text("Claude guidance")

    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
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

    assert request.provider == "gemini"
    assert [tool.name for tool in request.tools] == profile.tool_registry.names()
    assert request.tool_choice is not None
    assert request.tool_choice.mode == "auto"
    assert request.messages[0].role == unified_llm.Role.SYSTEM
    assert "AGENTS.md" in system_prompt
    assert "GEMINI.md" in system_prompt
    assert ".codex/instructions.md" not in system_prompt
    assert "CLAUDE.md" not in system_prompt

    client.released[0].set()
    await processing_task
    assert session.state == agent.SessionState.IDLE
    await session.close()
    assert session.state == agent.SessionState.CLOSED


@pytest.mark.asyncio
async def test_gemini_profile_latest_wins_custom_overrides_apply_through_the_registry(
    tmp_path: Path,
) -> None:
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
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
async def test_gemini_profile_subagent_tools_execute_through_the_registry(
    tmp_path: Path,
) -> None:
    client = _BlockingCompleteClient(
        [
            _assistant_response("child response 1", "resp-1"),
            _assistant_response("child response 2", "resp-2"),
        ]
    )
    profile = profiles.GeminiProviderProfile(model="gemini-3.1-pro-preview")
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
