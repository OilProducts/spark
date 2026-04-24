from __future__ import annotations

import logging
from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Protocol, runtime_checkable

logger = logging.getLogger(__name__)


@dataclass(slots=True)
class ExecResult:
    stdout: str = ""
    stderr: str = ""
    exit_code: int = 0
    timed_out: bool = False
    duration_ms: int = 0


@dataclass(slots=True)
class DirEntry:
    name: str
    is_dir: bool
    size: int | None = None


@dataclass(slots=True)
class GrepOptions:
    glob_filter: str | None = None
    case_insensitive: bool = False
    max_results: int = 100

    def __post_init__(self) -> None:
        if self.max_results < 1:
            raise ValueError("max_results must be at least 1")


class EnvironmentInheritancePolicy(StrEnum):
    INHERIT_ALL = "inherit_all"
    INHERIT_NONE = "inherit_none"
    INHERIT_CORE_ONLY = "inherit_core_only"


@runtime_checkable
class ExecutionEnvironment(Protocol):
    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str | bytes: ...

    def write_file(self, path: str | Path, content: str) -> None: ...

    def file_exists(self, path: str | Path) -> bool: ...

    def is_directory(self, path: str | Path) -> bool: ...

    def delete_file(self, path: str | Path) -> None: ...

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None: ...

    def list_directory(self, path: str | Path, depth: int) -> list[DirEntry]: ...

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: Mapping[str, str] | None = None,
    ) -> ExecResult: ...

    def grep(self, pattern: str, path: str | Path, options: GrepOptions) -> str: ...

    def glob(self, pattern: str, path: str | Path) -> list[str]: ...

    def initialize(self) -> None: ...

    def cleanup(self) -> None: ...

    def working_directory(self) -> str: ...

    def platform(self) -> str: ...

    def os_version(self) -> str: ...


__all__ = [
    "DirEntry",
    "EnvironmentInheritancePolicy",
    "ExecResult",
    "ExecutionEnvironment",
    "GrepOptions",
]
