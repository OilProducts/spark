from __future__ import annotations

from pathlib import Path

import pytest

import agent
import agent.builtin_tools as builtin_tools
import unified_llm


def _make_openai_session(
    tmp_path: Path,
    *,
    environment: object | None = None,
) -> agent.Session:
    execution_environment = (
        environment
        if environment is not None
        else agent.LocalExecutionEnvironment(working_dir=tmp_path)
    )
    profile = agent.ProviderProfile(id="openai", model="gpt-5.1")
    profile.tool_registry = builtin_tools.register_builtin_tools(provider_profile=profile)
    return agent.Session(profile=profile, execution_env=execution_environment)


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


class _ContractPatchEnvironment:
    def __init__(self) -> None:
        self.files: dict[str, str] = {}
        self.calls: list[tuple[str, ...]] = []
        self.initialized = False
        self.cleaned_up = False
        self._working_directory = "workspace"

    def _key(self, path: str | Path) -> str:
        return str(path)

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str:
        key = self._key(path)
        self.calls.append(("read_file", key))
        if key not in self.files:
            raise FileNotFoundError(key)
        content = self.files[key]
        if offset is not None and offset < 1:
            raise ValueError("offset must be at least 1")
        if limit is not None and limit < 0:
            raise ValueError("limit must be non-negative")
        if offset is None and limit is None:
            return content
        lines = content.splitlines(keepends=True)
        start = 0 if offset is None else offset - 1
        end = None if limit is None else start + limit
        return "".join(lines[start:end])

    def write_file(self, path: str | Path, content: str) -> None:
        key = self._key(path)
        self.calls.append(("write_file", key))
        self.files[key] = content

    def file_exists(self, path: str | Path) -> bool:
        key = self._key(path)
        self.calls.append(("file_exists", key))
        return key in self.files

    def is_directory(self, path: str | Path) -> bool:
        key = self._key(path)
        self.calls.append(("is_directory", key))
        return False

    def delete_file(self, path: str | Path) -> None:
        key = self._key(path)
        self.calls.append(("delete_file", key))
        if key not in self.files:
            raise FileNotFoundError(key)
        del self.files[key]

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        source_key = self._key(source_path)
        destination_key = self._key(destination_path)
        self.calls.append(("rename_file", source_key, destination_key))
        if source_key not in self.files:
            raise FileNotFoundError(source_key)
        if destination_key in self.files:
            raise FileExistsError(destination_key)
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
        self.calls.append(
            ("exec_command", command, "" if working_dir is None else str(working_dir))
        )
        return agent.ExecResult(
            stdout="",
            stderr="",
            exit_code=1,
            timed_out=False,
            duration_ms=1,
        )

    def grep(self, pattern: str, path: str | Path, options: agent.GrepOptions) -> str:
        return ""

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        return []

    def initialize(self) -> None:
        self.initialized = True

    def cleanup(self) -> None:
        self.cleaned_up = True

    def working_directory(self) -> str:
        return self._working_directory

    def platform(self) -> str:
        return "test"

    def os_version(self) -> str:
        return "1.0"


def test_openai_builtin_tool_registry_exposes_apply_patch() -> None:
    profile = agent.ProviderProfile(id="openai", model="gpt-5.1")

    registry = builtin_tools.register_builtin_tools(provider_profile=profile)

    assert "apply_patch" in registry.names()
    assert registry.get("apply_patch") is not None
    assert registry.get("apply_patch").definition.parameters == {
        "type": "object",
        "properties": {
            "patch": {"type": "string", "minLength": 1},
        },
        "required": ["patch"],
        "additionalProperties": False,
    }


@pytest.mark.asyncio
async def test_apply_patch_reports_parse_errors_without_touching_files(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("example.txt", "original\n")
    session = _make_openai_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": (
                "*** Update File: example.txt\n"
                "@@ example.txt\n"
                "- original\n"
                "+ changed\n"
                "*** End Patch"
            ),
        },
    )

    assert result.is_error is True
    assert result.content == "Patch parse error: missing *** Begin Patch marker"
    assert environment.read_file("example.txt") == "original\n"


@pytest.mark.asyncio
async def test_apply_patch_applies_add_delete_update_and_rename_with_multiple_hunks(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file(
        "src/app.py",
        "def main():\n    print(\"Hello\")\n    return 0\n# trailing\n",
    )
    environment.write_file(
        "src/old_name.py",
        "import os\nimport old_dep\n\ndef main():\n    return 0\n",
    )
    environment.write_file("src/remove_me.txt", "delete me\n")
    session = _make_openai_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Add File: src/new_file.py",
                    "+first line",
                    "+second line",
                    "*** Delete File: src/remove_me.txt",
                    "*** Update File: src/app.py",
                    "@@ def main():",
                    " def main():",
                    '     print("Hello")',
                    "-    return 0",
                    "+    return 1",
                    "@@ # trailing",
                    "-# trailing",
                    "+# updated trailing",
                    "*** Update File: src/old_name.py",
                    "*** Move to: src/new_name.py",
                    "@@ import old_dep",
                    " import os",
                    "-import old_dep",
                    "+import new_dep",
                    "*** End Patch",
                ]
            ),
        },
    )

    assert result.is_error is False
    assert result.content == [
        {"operation": "add", "path": "src/new_file.py"},
        {"operation": "delete", "path": "src/remove_me.txt"},
        {"operation": "update", "path": "src/app.py", "hunks": 2},
        {
            "operation": "update+rename",
            "path": "src/old_name.py",
            "new_path": "src/new_name.py",
            "hunks": 1,
        },
    ]
    assert environment.read_file("src/new_file.py") == "first line\nsecond line\n"
    assert not environment.file_exists("src/remove_me.txt")
    assert environment.read_file("src/app.py") == (
        "def main():\n    print(\"Hello\")\n    return 1\n# updated trailing\n"
    )
    assert environment.read_file("src/new_name.py") == (
        "import os\nimport new_dep\n\ndef main():\n    return 0\n"
    )
    assert not environment.file_exists("src/old_name.py")


@pytest.mark.asyncio
async def test_apply_patch_uses_context_hints_to_disambiguate_repeated_content(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file(
        "duplicate.py",
        (
            "def one():\n"
            "    value = \"alpha\"\n"
            "    return value\n"
            "\n"
            "def two():\n"
            "    value = \"alpha\"\n"
            "    return value\n"
        ),
    )
    session = _make_openai_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Update File: duplicate.py",
                    "@@ def two():",
                    "-    value = \"alpha\"",
                    "+    value = \"beta\"",
                    "*** End Patch",
                ]
            ),
        },
    )

    assert result.is_error is False
    assert result.content == [{"operation": "update", "path": "duplicate.py", "hunks": 1}]
    assert environment.read_file("duplicate.py") == (
        "def one():\n"
        "    value = \"alpha\"\n"
        "    return value\n"
        "\n"
        "def two():\n"
        "    value = \"beta\"\n"
        "    return value\n"
    )


@pytest.mark.asyncio
async def test_apply_patch_uses_fuzzy_matching_for_whitespace_and_unicode_punctuation(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("notes.txt", 'answer = "Cost – benefit"\n')
    session = _make_openai_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Update File: notes.txt",
                    '@@ answer = "Cost - benefit"',
                    '-answer   =   "Cost - benefit"',
                    '+answer = "Cost - benefit!"',
                    "*** End Patch",
                ]
            ),
        },
    )

    assert result.is_error is False
    assert result.content == [{"operation": "update", "path": "notes.txt", "hunks": 1}]
    assert environment.read_file("notes.txt") == 'answer = "Cost - benefit!"\n'


@pytest.mark.asyncio
async def test_apply_patch_reports_missing_file_and_verification_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("verify.txt", "old\n")
    session = _make_openai_session(tmp_path, environment=environment)

    missing_result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Update File: missing.txt",
                    "@@ missing.txt",
                    "-missing",
                    "+present",
                    "*** End Patch",
                ]
            ),
        },
        tool_call_id="call-2",
    )
    assert missing_result.is_error is True
    assert missing_result.content == "File not found: missing.txt"

    def _noop_write_file(path: str | Path, content: str) -> None:
        return None

    monkeypatch.setattr(environment, "write_file", _noop_write_file)

    verification_result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Update File: verify.txt",
                    "@@ verify.txt",
                    "-old",
                    "+new",
                    "*** End Patch",
                ]
            ),
        },
        tool_call_id="call-3",
    )

    assert verification_result.is_error is True
    assert verification_result.content == "Patch verification failed: verify.txt"
    assert environment.read_file("verify.txt") == "old\n"


@pytest.mark.asyncio
async def test_apply_patch_routes_filesystem_operations_through_the_execution_environment_contract(
    tmp_path: Path,
) -> None:
    environment = _ContractPatchEnvironment()
    environment.files.update(
        {
            "src/app.py": "def main():\n    print(\"Hello\")\n    return 0\n# trailing\n",
            "src/old_name.py": "import os\nimport old_dep\n\ndef main():\n    return 0\n",
            "src/remove_me.txt": "delete me\n",
        }
    )
    session = _make_openai_session(tmp_path, environment=environment)

    result = await _execute_tool(
        session,
        "apply_patch",
        {
            "patch": "\n".join(
                [
                    "*** Begin Patch",
                    "*** Add File: src/new_file.py",
                    "+first line",
                    "+second line",
                    "*** Delete File: src/remove_me.txt",
                    "*** Update File: src/app.py",
                    "@@ def main():",
                    " def main():",
                    '     print("Hello")',
                    "-    return 0",
                    "+    return 1",
                    "@@ # trailing",
                    "-# trailing",
                    "+# updated trailing",
                    "*** Update File: src/old_name.py",
                    "*** Move to: src/new_name.py",
                    "@@ import old_dep",
                    " import os",
                    "-import old_dep",
                    "+import new_dep",
                    "*** End Patch",
                ]
            ),
        },
    )

    assert result.is_error is False
    assert result.content == [
        {"operation": "add", "path": "src/new_file.py"},
        {"operation": "delete", "path": "src/remove_me.txt"},
        {"operation": "update", "path": "src/app.py", "hunks": 2},
        {
            "operation": "update+rename",
            "path": "src/old_name.py",
            "new_path": "src/new_name.py",
            "hunks": 1,
        },
    ]
    assert environment.files == {
        "src/app.py": "def main():\n    print(\"Hello\")\n    return 1\n# updated trailing\n",
        "src/new_file.py": "first line\nsecond line\n",
        "src/new_name.py": "import os\nimport new_dep\n\ndef main():\n    return 0\n",
    }
    assert ("delete_file", "src/remove_me.txt") in environment.calls
    assert ("rename_file", "src/old_name.py", "src/new_name.py") in environment.calls
    assert ("read_file", "src/app.py") in environment.calls
    assert ("read_file", "src/new_name.py") in environment.calls
