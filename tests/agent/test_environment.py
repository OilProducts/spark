from __future__ import annotations

from pathlib import Path

import agent


class _MemoryEnvironment:
    def __init__(self) -> None:
        self.files: dict[str, str] = {}
        self.initialized = False
        self.cleaned_up = False

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str:
        content = self.files[str(path)]
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
        key = str(path)
        if key not in self.files:
            raise FileNotFoundError(key)
        del self.files[key]

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        source_key = str(source_path)
        destination_key = str(destination_path)
        if source_key not in self.files:
            raise FileNotFoundError(source_key)
        if destination_key in self.files:
            raise FileExistsError(destination_key)
        self.files[destination_key] = self.files.pop(source_key)

    def list_directory(self, path: str | Path, depth: int) -> list[agent.DirEntry]:
        return [agent.DirEntry(name=str(path), is_dir=False, size=len(self.files[str(path)]))]

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: dict[str, str] | None = None,
    ) -> agent.ExecResult:
        return agent.ExecResult(
            stdout=command,
            stderr="",
            exit_code=0,
            timed_out=False,
            duration_ms=1,
        )

    def grep(self, pattern: str, path: str | Path, options: agent.GrepOptions) -> str:
        return f"{path}:1:{pattern}"

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        return [str(path)]

    def initialize(self) -> None:
        self.initialized = True

    def cleanup(self) -> None:
        self.cleaned_up = True

    def working_directory(self) -> str:
        return "."

    def platform(self) -> str:
        return "test"

    def os_version(self) -> str:
        return "1.0"


def test_execution_environment_protocol_accepts_structural_implementations() -> None:
    environment = _MemoryEnvironment()

    assert isinstance(environment, agent.ExecutionEnvironment)
    environment.initialize()
    environment.write_file("notes.txt", "alpha\nbeta\n")

    assert environment.file_exists("notes.txt") is True
    assert environment.is_directory("notes.txt") is False
    assert environment.read_file("notes.txt", offset=2, limit=1) == "beta\n"
    environment.write_file("notes-2.txt", "gamma\n")
    environment.rename_file("notes-2.txt", "renamed.txt")
    assert environment.file_exists("notes-2.txt") is False
    assert environment.file_exists("renamed.txt") is True
    environment.delete_file("renamed.txt")
    assert environment.file_exists("renamed.txt") is False
    assert environment.list_directory("notes.txt", depth=0) == [
        agent.DirEntry(name="notes.txt", is_dir=False, size=11)
    ]
    assert environment.exec_command("echo hello").stdout == "echo hello"
    assert environment.grep("alpha", "notes.txt", agent.GrepOptions()) == "notes.txt:1:alpha"
    assert environment.glob("*.txt", "notes.txt") == ["notes.txt"]
    assert environment.working_directory() == "."
    assert environment.platform() == "test"
    assert environment.os_version() == "1.0"
    environment.cleanup()
    assert environment.initialized is True
    assert environment.cleaned_up is True


def test_environment_records_and_policy_types_are_public() -> None:
    result = agent.ExecResult(
        stdout="out",
        stderr="err",
        exit_code=17,
        timed_out=True,
        duration_ms=42,
    )
    entry = agent.DirEntry(name="src", is_dir=True)
    options = agent.GrepOptions(glob_filter="*.py", case_insensitive=True, max_results=3)

    assert result.stdout == "out"
    assert result.stderr == "err"
    assert result.exit_code == 17
    assert result.timed_out is True
    assert result.duration_ms == 42
    assert entry.name == "src"
    assert entry.is_dir is True
    assert entry.size is None
    assert options.glob_filter == "*.py"
    assert options.case_insensitive is True
    assert options.max_results == 3
    assert [member.value for member in agent.EnvironmentInheritancePolicy] == [
        "inherit_all",
        "inherit_none",
        "inherit_core_only",
    ]
