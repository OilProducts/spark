from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest

import agent
import agent.builtin_tools as builtin_tools
import unified_llm as unified_llm

PNG_BYTES = (
    b"\x89PNG\r\n\x1a\n"
    b"\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01"
    b"\x08\x06\x00\x00\x00\x1f\x15\xc4\x89"
    b"\x00\x00\x00\x0cIDAT\x08\xd7c\xf8\x0f\x00\x01\x01\x01\x00\x18\xdd\x8d\x18"
    b"\x00\x00\x00\x00IEND\xaeB`\x82"
)


def _make_session(
    tmp_path: Path,
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
        id="test-provider",
        model="test-model",
        capabilities=capabilities or {},
    )
    profile.tool_registry = builtin_tools.register_builtin_tools(provider_profile=profile)
    return agent.Session(profile=profile, execution_env=execution_environment)


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
                exit_code=1,
                timed_out=False,
                duration_ms=1,
            )
        )
        self.exec_calls: list[dict[str, object | None]] = []
        self.grep_calls: list[dict[str, object]] = []
        self.glob_calls: list[dict[str, object]] = []
        self.grep_result = ""
        self.glob_result: list[str] = []
        self.grep_error: Exception | None = None
        self.glob_error: Exception | None = None

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str:
        raise FileNotFoundError(path)

    def write_file(self, path: str | Path, content: str) -> None:
        return None

    def file_exists(self, path: str | Path) -> bool:
        return False

    def is_directory(self, path: str | Path) -> bool:
        return False

    def delete_file(self, path: str | Path) -> None:
        return None

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        return None

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
        if self.grep_error is not None:
            raise self.grep_error
        return self.grep_result

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        self.glob_calls.append(
            {
                "pattern": pattern,
                "path": path,
            }
        )
        if self.glob_error is not None:
            raise self.glob_error
        return list(self.glob_result)

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


def test_builtin_tool_registry_exposes_expected_shared_tools() -> None:
    registry = builtin_tools.build_builtin_tool_registry()

    assert registry.names() == [
        "read_file",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "read_many_files",
        "list_dir",
    ]


def test_builtin_file_tool_registry_exposes_expected_file_tools() -> None:
    registry = builtin_tools.build_builtin_file_tool_registry()

    assert registry.names() == [
        "read_file",
        "write_file",
        "edit_file",
        "read_many_files",
        "list_dir",
    ]


def test_builtin_file_tool_definitions_are_shared_subset_of_builtin_definitions() -> None:
    builtin_definitions = builtin_tools.builtin_tool_definitions()
    builtin_definition_map = {
        definition.name: definition for definition in builtin_definitions
    }

    file_definitions = builtin_tools.builtin_file_tool_definitions()

    assert [definition.name for definition in file_definitions] == [
        "read_file",
        "write_file",
        "edit_file",
        "read_many_files",
        "list_dir",
    ]
    for definition in file_definitions:
        shared_definition = builtin_definition_map[definition.name]
        assert definition.description == shared_definition.description
        assert definition.parameters == shared_definition.parameters


def test_builtin_tool_helpers_are_exported_from_agent_package() -> None:
    shared_tool_names = [
        "read_file",
        "write_file",
        "edit_file",
        "shell",
        "grep",
        "glob",
        "read_many_files",
        "list_dir",
    ]
    file_tool_names = [
        "read_file",
        "write_file",
        "edit_file",
        "read_many_files",
        "list_dir",
    ]

    assert agent.build_builtin_tool_registry().names() == shared_tool_names
    assert agent.register_builtin_tools().names() == shared_tool_names
    assert [
        definition.name for definition in agent.builtin_tool_definitions()
    ] == shared_tool_names

    assert agent.build_builtin_file_tool_registry().names() == file_tool_names
    assert agent.register_builtin_file_tools().names() == file_tool_names
    assert agent.register_file_tools().names() == file_tool_names
    assert agent.build_file_tool_registry().names() == file_tool_names
    assert [
        definition.name for definition in agent.builtin_file_tool_definitions()
    ] == file_tool_names
    assert [
        definition.name for definition in agent.file_tool_definitions()
    ] == file_tool_names


@pytest.mark.asyncio
async def test_read_file_formats_text_and_honors_offset_and_default_limit(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("notes.txt", "alpha\nbeta\ngamma\n")
    long_lines = "\n".join(f"line-{index:04d}" for index in range(1, 2002))
    environment.write_file("long.txt", f"{long_lines}\n")
    session = _make_session(tmp_path, environment=environment)

    offset_result = await _execute_tool(
        session,
        "read_file",
        {"path": "notes.txt", "offset": 2, "limit": 1},
    )
    assert offset_result.is_error is False
    assert offset_result.content == "002 | beta"

    limit_result = await _execute_tool(
        session,
        "read_file",
        {"path": "long.txt"},
        tool_call_id="call-2",
    )
    assert limit_result.is_error is False
    limit_lines = limit_result.content.splitlines()
    assert len(limit_lines) == 2000
    assert limit_lines[0] == "001 | line-0001"
    assert limit_lines[-1] == "2000 | line-2000"


@pytest.mark.asyncio
async def test_read_file_reports_permission_errors_via_the_tool_pipeline(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    session = _make_session(tmp_path, environment=environment)

    def _raise_permission_error(
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str:
        raise PermissionError("denied")

    monkeypatch.setattr(environment, "read_file", _raise_permission_error)

    result = await _execute_tool(
        session,
        "read_file",
        {"path": "secret.txt"},
    )

    assert result.is_error is True
    assert result.content == "Permission denied: secret.txt"


@pytest.mark.asyncio
async def test_read_file_gates_binary_images_on_multimodal_support(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    image_path = tmp_path / "image.png"
    image_path.write_bytes(PNG_BYTES)

    no_multimodal_session = _make_session(tmp_path, environment=environment)
    denied_result = await _execute_tool(
        no_multimodal_session,
        "read_file",
        {"path": "image.png"},
    )
    assert denied_result.is_error is True
    assert denied_result.content == "Binary file not supported: image.png"

    multimodal_session = _make_session(
        tmp_path,
        environment=environment,
        capabilities={"vision": True},
    )
    allowed_result = await _execute_tool(
        multimodal_session,
        "read_file",
        {"path": "image.png"},
        tool_call_id="call-2",
    )

    assert allowed_result.is_error is False
    assert allowed_result.image_data == PNG_BYTES
    assert allowed_result.image_media_type == "image/png"
    assert "image.png" in allowed_result.content


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("arguments", "expected_message"),
    [
        (
            {"path": "missing.txt"},
            "File not found: missing.txt",
        ),
        (
            {"path": "folder"},
            "Is a directory: folder",
        ),
        (
            {"path": "binary.dat"},
            "Binary file not supported: binary.dat",
        ),
    ],
)
async def test_read_file_reports_recoverable_tool_errors_via_tool_registry(
    tmp_path: Path,
    arguments: dict[str, object],
    expected_message: str,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("folder/placeholder.txt", "inside\n")
    environment.write_file("binary.dat", "alpha\x00beta\n")
    session = _make_session(
        tmp_path,
        environment=environment,
        capabilities={"vision": True},
    )

    result = await _execute_tool(session, "read_file", arguments)

    assert result.is_error is True
    assert result.content == expected_message


@pytest.mark.asyncio
async def test_write_file_and_edit_file_report_bytes_written_and_apply_exact_replacements(
    tmp_path: Path,
) -> None:
    session = _make_session(tmp_path)

    write_result = await _execute_tool(
        session,
        "write_file",
        {"path": "nested/output.txt", "content": "alpha\nbeta\n"},
    )
    assert write_result.is_error is False
    assert write_result.content == {
        "path": "nested/output.txt",
        "bytes_written": 11,
    }
    assert session.execution_environment.read_file("nested/output.txt") == "alpha\nbeta\n"

    edit_result = await _execute_tool(
        session,
        "edit_file",
        {
            "path": "nested/output.txt",
            "old_string": "beta",
            "new_string": "gamma",
        },
        tool_call_id="call-2",
    )
    assert edit_result.is_error is False
    assert edit_result.content["path"] == "nested/output.txt"
    assert edit_result.content["replacements"] == 1
    assert edit_result.content["replace_all"] is False
    assert session.execution_environment.read_file("nested/output.txt") == "alpha\ngamma\n"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "content, arguments, expected_message",
    [
        (
            "alpha\nbeta\nalpha\n",
            {
                "path": "duplicate.txt",
                "old_string": "alpha",
                "new_string": "omega",
            },
            "old_string is not unique in duplicate.txt: 2 matches",
        ),
        (
            "alpha\nbeta\n",
            {
                "path": "missing.txt",
                "old_string": "delta",
                "new_string": "omega",
            },
            "old_string not found in missing.txt",
        ),
    ],
)
async def test_edit_file_reports_conflicts_without_applying_changes(
    tmp_path: Path,
    content: str,
    arguments: dict[str, object],
    expected_message: str,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file(arguments["path"], content)
    session = _make_session(tmp_path, environment=environment)

    result = await _execute_tool(session, "edit_file", arguments)

    assert result.is_error is True
    assert result.content == expected_message
    assert environment.read_file(arguments["path"]) == content


@pytest.mark.asyncio
async def test_read_many_files_and_list_dir_return_structured_content(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("alpha.txt", "first\nsecond\n")
    environment.write_file("nested/beta.txt", "third\n")
    session = _make_session(tmp_path, environment=environment)

    many_result = await _execute_tool(
        session,
        "read_many_files",
        {"paths": ["alpha.txt", "nested/beta.txt"]},
    )
    assert many_result.is_error is False
    assert many_result.content["count"] == 2
    assert many_result.content["files"][0]["path"] == "alpha.txt"
    assert many_result.content["files"][0]["content"].splitlines()[0] == "001 | first"
    assert many_result.content["files"][1]["path"] == "nested/beta.txt"
    assert many_result.content["files"][1]["content"].splitlines()[0] == "001 | third"

    dir_result = await _execute_tool(
        session,
        "list_dir",
        {"path": ".", "depth": 1},
        tool_call_id="call-2",
    )
    assert dir_result.is_error is False
    entries = {entry["name"]: entry for entry in dir_result.content["entries"]}
    assert entries["alpha.txt"] == {
        "name": "alpha.txt",
        "is_dir": False,
        "size": len("first\nsecond\n"),
    }
    assert entries["nested"] == {
        "name": "nested",
        "is_dir": True,
        "size": None,
    }
    assert entries["nested/beta.txt"] == {
        "name": "nested/beta.txt",
        "is_dir": False,
        "size": len("third\n"),
    }


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("provider_options_map", "arguments", "session_config", "expected_timeout_ms"),
    [
        (
            {"shell_timeout_ms": 300},
            {"command": "echo hello"},
            agent.SessionConfig(default_command_timeout_ms=75, max_command_timeout_ms=200),
            200,
        ),
        (
            {},
            {"command": "echo hello"},
            agent.SessionConfig(default_command_timeout_ms=75, max_command_timeout_ms=200),
            75,
        ),
        (
            {"shell_timeout_ms": 300},
            {"command": "echo hello", "timeout_ms": 40},
            agent.SessionConfig(default_command_timeout_ms=75, max_command_timeout_ms=200),
            40,
        ),
        (
            {"shell_timeout_ms": 300},
            {"command": "echo hello", "timeout_ms": 400},
            agent.SessionConfig(default_command_timeout_ms=75, max_command_timeout_ms=200),
            200,
        ),
    ],
)
async def test_shell_uses_provider_defaults_explicit_timeouts_and_session_bounds(
    tmp_path: Path,
    provider_options_map: dict[str, object],
    arguments: dict[str, object],
    session_config: agent.SessionConfig,
    expected_timeout_ms: int,
) -> None:
    environment = _RecordingToolEnvironment()
    profile = agent.ProviderProfile(
        id="test-provider",
        model="test-model",
        provider_options_map=provider_options_map,
    )
    profile.tool_registry = builtin_tools.register_builtin_tools(provider_profile=profile)
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=session_config,
    )
    environment.exec_result = agent.ExecResult(
        stdout="stdout",
        stderr="stderr",
        exit_code=0,
        timed_out=False,
        duration_ms=17,
    )
    environment.exec_calls.clear()

    result = await _execute_tool(session, "shell", arguments)

    assert environment.exec_calls == [
        {
            "command": "echo hello",
            "timeout_ms": expected_timeout_ms,
            "working_dir": None,
            "env_vars": None,
        }
    ]
    assert result.is_error is False
    assert result.content == {
        "stdout": "stdout",
        "stderr": "stderr",
        "exit_code": 0,
        "timed_out": False,
        "duration_ms": 17,
    }


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("exec_result", "expected_content"),
    [
        (
            agent.ExecResult(
                stdout="partial",
                stderr="boom",
                exit_code=7,
                timed_out=False,
                duration_ms=12,
            ),
            {
                "stdout": "partial",
                "stderr": "boom",
                "exit_code": 7,
                "timed_out": False,
                "duration_ms": 12,
            },
        ),
        (
            agent.ExecResult(
                stdout="start",
                stderr="Command timed out after 50 ms",
                exit_code=124,
                timed_out=True,
                duration_ms=53,
            ),
            {
                "stdout": "start",
                "stderr": "Command timed out after 50 ms",
                "exit_code": 124,
                "timed_out": True,
                "duration_ms": 53,
            },
        ),
    ],
)
async def test_shell_reports_recoverable_nonzero_exit_and_timeout_outcomes(
    tmp_path: Path,
    exec_result: agent.ExecResult,
    expected_content: dict[str, object],
) -> None:
    environment = _RecordingToolEnvironment()
    profile = agent.ProviderProfile(id="test-provider", model="test-model")
    profile.tool_registry = builtin_tools.register_builtin_tools(provider_profile=profile)
    session = agent.Session(
        profile=profile,
        execution_env=environment,
        config=agent.SessionConfig(default_command_timeout_ms=75, max_command_timeout_ms=200),
    )
    environment.exec_result = exec_result
    environment.exec_calls.clear()

    result = await _execute_tool(
        session,
        "shell",
        {"command": "echo hello", "timeout_ms": 50},
    )

    assert environment.exec_calls == [
        {
            "command": "echo hello",
            "timeout_ms": 50,
            "working_dir": None,
            "env_vars": None,
        }
    ]
    assert result.is_error is True
    assert result.content == expected_content


@pytest.mark.asyncio
async def test_shell_accepts_description_through_public_tool_execution_path(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment(
        exec_result=agent.ExecResult(
            stdout="stdout",
            stderr="stderr",
            exit_code=0,
            timed_out=False,
            duration_ms=17,
        )
    )
    session = _make_session(tmp_path, environment=environment)
    environment.exec_calls.clear()

    result = await _execute_tool(
        session,
        "shell",
        {
            "command": "echo hello",
            "timeout_ms": 50,
            "description": "Capture the command output",
        },
    )

    assert environment.exec_calls == [
        {
            "command": "echo hello",
            "timeout_ms": 50,
            "working_dir": None,
            "env_vars": None,
        }
    ]
    assert result.is_error is False
    assert result.content == {
        "stdout": "stdout",
        "stderr": "stderr",
        "exit_code": 0,
        "timed_out": False,
        "duration_ms": 17,
    }


@pytest.mark.asyncio
async def test_grep_returns_structured_matches_and_uses_default_path_when_omitted(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment()
    environment.grep_result = (
        "workspace/src/app.py:3:alpha:beta\n"
        "workspace/nested/tool.py:8:gamma\n"
    )
    session = _make_session(tmp_path, environment=environment)
    environment.grep_calls.clear()

    result = await _execute_tool(
        session,
        "grep",
        {
            "pattern": "alpha",
            "glob_filter": "*.py",
            "case_insensitive": True,
        },
    )

    assert environment.grep_calls == [
        {
            "pattern": "alpha",
            "path": ".",
            "options": agent.GrepOptions(
                glob_filter="*.py",
                case_insensitive=True,
                max_results=100,
            ),
        }
    ]
    assert result.is_error is False
    assert result.content == {
        "matches": [
            {
                "path": "workspace/src/app.py",
                "line_number": 3,
                "line": "alpha:beta",
            },
            {
                "path": "workspace/nested/tool.py",
                "line_number": 8,
                "line": "gamma",
            },
        ]
    }


@pytest.mark.asyncio
async def test_grep_reports_invalid_regex_as_recoverable_error(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment()
    environment.grep_error = ValueError("missing ), unterminated subpattern at position 0")
    session = _make_session(tmp_path, environment=environment)
    environment.grep_calls.clear()

    result = await _execute_tool(session, "grep", {"pattern": "("})

    assert environment.grep_calls == [
        {
            "pattern": "(",
            "path": ".",
            "options": agent.GrepOptions(
                glob_filter=None,
                case_insensitive=False,
                max_results=100,
            ),
        }
    ]
    assert result.is_error is True
    assert result.content == (
        "Invalid regex pattern: missing ), unterminated subpattern at position 0"
    )


def test_search_tools_report_missing_path_directly() -> None:
    environment = _RecordingToolEnvironment()

    grep_result = builtin_tools.grep(
        {"pattern": "alpha", "path": ""},
        environment,
    )
    glob_result = builtin_tools.glob(
        {"pattern": "*.txt", "path": ""},
        environment,
    )

    assert grep_result.is_error is True
    assert grep_result.content == "Missing required argument: path"
    assert glob_result.is_error is True
    assert glob_result.content == "Missing required argument: path"
    assert environment.grep_calls == []
    assert environment.glob_calls == []


@pytest.mark.asyncio
async def test_glob_returns_newest_first_paths_and_reports_invalid_pattern(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment()
    environment.glob_result = ["omega.txt", "alpha.txt"]
    session = _make_session(tmp_path, environment=environment)
    environment.glob_calls.clear()

    result = await _execute_tool(session, "glob", {"pattern": "*.txt"})

    assert environment.glob_calls == [
        {
            "pattern": "*.txt",
            "path": ".",
        }
    ]
    assert result.is_error is False
    assert result.content == ["omega.txt", "alpha.txt"]


@pytest.mark.asyncio
async def test_glob_reports_invalid_pattern_as_recoverable_error(
    tmp_path: Path,
) -> None:
    environment = _RecordingToolEnvironment()
    environment.glob_error = ValueError("unexpected end of pattern")
    session = _make_session(tmp_path, environment=environment)
    environment.glob_calls.clear()

    result = await _execute_tool(session, "glob", {"pattern": "["})

    assert environment.glob_calls == [
        {
            "pattern": "[",
            "path": ".",
        }
    ]
    assert result.is_error is True
    assert result.content == "Invalid glob pattern: unexpected end of pattern"


@pytest.mark.asyncio
async def test_grep_defaults_to_active_working_directory_for_relative_local_environment(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(shutil, "which", lambda name: None)
    environment = agent.LocalExecutionEnvironment(working_dir="workspace")
    environment.write_file("notes.txt", "alpha\nbeta\nALPHA\n")
    environment.write_file("nested/other.txt", "alpha\n")
    session = _make_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "grep",
        {"pattern": "alpha", "case_insensitive": True},
    )

    assert result.is_error is False
    assert result.content == {
        "matches": [
            {
                "path": "nested/other.txt",
                "line_number": 1,
                "line": "alpha",
            },
            {
                "path": "notes.txt",
                "line_number": 1,
                "line": "alpha",
            },
            {
                "path": "notes.txt",
                "line_number": 3,
                "line": "ALPHA",
            },
        ]
    }


@pytest.mark.asyncio
async def test_glob_defaults_to_active_working_directory_for_relative_local_environment(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(tmp_path)
    environment = agent.LocalExecutionEnvironment(working_dir="workspace")
    old_file = tmp_path / "workspace" / "alpha.txt"
    new_file = tmp_path / "workspace" / "omega.txt"
    environment.write_file("alpha.txt", "old")
    environment.write_file("omega.txt", "new")
    os.utime(old_file, (1, 1))
    os.utime(new_file, (2, 2))
    session = _make_session(tmp_path, environment=environment)

    result = await _execute_tool(session, "glob", {"pattern": "*.txt"})

    assert result.is_error is False
    assert result.content == ["omega.txt", "alpha.txt"]


@pytest.mark.asyncio
async def test_glob_reports_absolute_patterns_as_recoverable_errors(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(tmp_path)
    environment = agent.LocalExecutionEnvironment(working_dir="workspace")
    environment.write_file("alpha.txt", "old")
    session = _make_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "glob",
        {"pattern": str(tmp_path / "*.txt")},
    )

    assert result.is_error is True
    assert result.content.startswith("Invalid glob pattern:")
