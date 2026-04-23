from __future__ import annotations

import fnmatch
import json
import logging
import os
import platform as platform_module
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from collections.abc import Mapping
from pathlib import Path

from .environment import (
    DirEntry,
    EnvironmentInheritancePolicy,
    ExecResult,
    GrepOptions,
)

logger = logging.getLogger(__name__)

_SENSITIVE_ENV_PATTERN = re.compile(
    r".*(?:_API_KEY|_SECRET|_TOKEN|_PASSWORD|_CREDENTIAL)$",
    re.IGNORECASE,
)
_CORE_ENV_KEYS = {
    "APPDATA",
    "CARGO_HOME",
    "CLASSPATH",
    "COMSPEC",
    "CONDA_PREFIX",
    "DYLD_LIBRARY_PATH",
    "GOBIN",
    "GOCACHE",
    "GOMODCACHE",
    "GOPATH",
    "GRADLE_HOME",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "JAVA_HOME",
    "LANG",
    "LD_LIBRARY_PATH",
    "LOCALAPPDATA",
    "LOGNAME",
    "MANPATH",
    "NVM_DIR",
    "NODE_PATH",
    "OLDPWD",
    "PATH",
    "PNPM_HOME",
    "PKG_CONFIG_PATH",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PYTHONHOME",
    "PYTHONPATH",
    "PWD",
    "RUSTUP_HOME",
    "SDKMAN_DIR",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
    "USERNAME",
    "UV_CACHE_DIR",
    "UV_TOOL_DIR",
    "VIRTUAL_ENV",
    "WINDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "YARN_CACHE_FOLDER",
}

def _coerce_policy(
    value: EnvironmentInheritancePolicy | str | None,
) -> EnvironmentInheritancePolicy:
    if value is None:
        return EnvironmentInheritancePolicy.INHERIT_CORE_ONLY
    if isinstance(value, EnvironmentInheritancePolicy):
        return value
    try:
        return EnvironmentInheritancePolicy(value)
    except ValueError:
        return EnvironmentInheritancePolicy[value]


def _is_sensitive_env_key(key: str) -> bool:
    return bool(_SENSITIVE_ENV_PATTERN.fullmatch(key))


def _is_core_env_key(key: str) -> bool:
    upper_key = key.upper()
    if upper_key in _CORE_ENV_KEYS:
        return True
    return upper_key.endswith(("_PATH", "_HOME", "_DIR"))


def _coerce_output(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="surrogateescape")
    return value


class LocalExecutionEnvironment:
    def __init__(
        self,
        working_dir: str | Path | None = None,
        *,
        working_directory: str | Path | None = None,
        default_command_timeout_ms: int = 10000,
        max_command_timeout_ms: int = 600000,
        environment_inheritance_policy: EnvironmentInheritancePolicy | str | None = None,
    ) -> None:
        if working_dir is not None and working_directory is not None:
            if Path(working_dir) != Path(working_directory):
                raise ValueError("working_dir and working_directory must match when both are set")
        configured = working_directory if working_directory is not None else working_dir
        if configured is None:
            configured = Path.cwd()
        self._configured_working_directory = Path(configured)
        self._resolved_working_directory = self._configured_working_directory.expanduser().resolve(
            strict=False
        )
        self._default_command_timeout_ms = default_command_timeout_ms
        self._max_command_timeout_ms = max_command_timeout_ms
        self._environment_inheritance_policy = _coerce_policy(environment_inheritance_policy)
        self._active_processes: set[subprocess.Popen[str]] = set()
        self._process_lock = threading.Lock()
        self._cleanup_started = False

    def initialize(self) -> None:
        self._resolved_working_directory.mkdir(parents=True, exist_ok=True)

    def cleanup(self) -> None:
        logger.debug("cleanup called for local execution environment")
        with self._process_lock:
            self._cleanup_started = True
            active_processes = [
                proc for proc in self._active_processes if proc.poll() is None
            ]

        if not active_processes:
            return

        logger.debug(
            "terminating %d active process group(s) during cleanup",
            len(active_processes),
        )
        for proc in active_processes:
            self._terminate_process(proc)

    def working_directory(self) -> str:
        return str(self._configured_working_directory)

    def with_working_directory(
        self,
        working_dir: str | Path | None = None,
    ) -> LocalExecutionEnvironment:
        configured_working_dir = (
            self._configured_working_directory if working_dir is None else working_dir
        )
        return LocalExecutionEnvironment(
            working_dir=configured_working_dir,
            default_command_timeout_ms=self._default_command_timeout_ms,
            max_command_timeout_ms=self._max_command_timeout_ms,
            environment_inheritance_policy=self._environment_inheritance_policy,
        )

    def platform(self) -> str:
        if sys.platform.startswith("win"):
            return "windows"
        if sys.platform == "darwin":
            return "darwin"
        if sys.platform in {"emscripten", "wasi"}:
            return "wasm"
        return "linux"

    def os_version(self) -> str:
        return platform_module.platform()

    def _resolve_path(self, value: str | Path) -> Path:
        candidate = Path(value)
        if not candidate.is_absolute():
            candidate = self._resolved_working_directory / candidate
        return candidate.expanduser().resolve(strict=False)

    def _resolve_command_cwd(self, value: str | Path | None) -> Path:
        if value is None:
            cwd = self._resolved_working_directory
        else:
            cwd = self._resolve_path(value)
        if not cwd.exists():
            raise FileNotFoundError(f"working directory does not exist: {cwd}")
        if not cwd.is_dir():
            raise NotADirectoryError(f"working directory is not a directory: {cwd}")
        return cwd

    def _display_path(self, path: Path) -> str:
        try:
            return str(path.relative_to(self._resolved_working_directory))
        except ValueError:
            return str(path)

    def _display_search_path(self, path_text: str) -> str:
        return self._display_path(self._resolve_path(path_text))

    def read_file(
        self,
        path: str | Path,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str | bytes:
        target = self._resolve_path(path)
        if not target.exists():
            raise FileNotFoundError(target)
        if target.is_dir():
            raise IsADirectoryError(target)
        if offset is not None and offset < 1:
            raise ValueError("offset must be at least 1")
        if limit is not None and limit < 0:
            raise ValueError("limit must be non-negative")

        raw_content = target.read_bytes()
        try:
            content = raw_content.decode("utf-8")
        except UnicodeDecodeError:
            return raw_content
        if offset is None and limit is None:
            return content
        lines = content.splitlines(keepends=True)
        start = 0 if offset is None else offset - 1
        end = None if limit is None else start + limit
        return "".join(lines[start:end])

    def write_file(self, path: str | Path, content: str) -> None:
        target = self._resolve_path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8", errors="surrogateescape")

    def file_exists(self, path: str | Path) -> bool:
        return self._resolve_path(path).exists()

    def is_directory(self, path: str | Path) -> bool:
        return self._resolve_path(path).is_dir()

    def delete_file(self, path: str | Path) -> None:
        target = self._resolve_path(path)
        if not target.exists():
            raise FileNotFoundError(target)
        if target.is_dir():
            raise IsADirectoryError(target)

        try:
            target.unlink()
        except FileNotFoundError as exc:
            raise FileNotFoundError(target) from exc
        except IsADirectoryError as exc:
            raise IsADirectoryError(target) from exc
        except PermissionError as exc:
            raise PermissionError(target) from exc

    def rename_file(self, source_path: str | Path, destination_path: str | Path) -> None:
        source = self._resolve_path(source_path)
        destination = self._resolve_path(destination_path)
        if not source.exists():
            raise FileNotFoundError(source)
        if source.is_dir():
            raise IsADirectoryError(source)
        if destination.exists():
            raise FileExistsError(destination)

        destination.parent.mkdir(parents=True, exist_ok=True)
        try:
            source.replace(destination)
        except FileNotFoundError as exc:
            raise FileNotFoundError(source) from exc
        except IsADirectoryError as exc:
            raise IsADirectoryError(source) from exc
        except FileExistsError as exc:
            raise FileExistsError(destination) from exc
        except PermissionError as exc:
            raise PermissionError(source) from exc

    def list_directory(self, path: str | Path, depth: int) -> list[DirEntry]:
        if depth < 0:
            raise ValueError("depth must be non-negative")
        base = self._resolve_path(path)
        if not base.exists():
            raise FileNotFoundError(base)
        if not base.is_dir():
            raise NotADirectoryError(base)

        entries: list[DirEntry] = []

        def walk(current: Path, remaining_depth: int, relative_prefix: Path) -> None:
            for child in sorted(current.iterdir(), key=lambda candidate: candidate.name):
                relative_name = relative_prefix / child.name
                is_dir = child.is_dir()
                size = None if is_dir else child.stat().st_size
                entries.append(
                    DirEntry(name=str(relative_name), is_dir=is_dir, size=size)
                )
                if is_dir and remaining_depth > 0:
                    walk(child, remaining_depth - 1, relative_name)

        walk(base, depth, Path(""))
        return entries

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: Mapping[str, str] | None = None,
    ) -> ExecResult:
        effective_timeout_ms = (
            self._default_command_timeout_ms if timeout_ms is None else timeout_ms
        )
        effective_timeout_ms = min(effective_timeout_ms, self._max_command_timeout_ms)
        if effective_timeout_ms < 1:
            raise ValueError("timeout_ms must be at least 1")

        cwd = self._resolve_command_cwd(working_dir)
        env = self._build_environment(env_vars)
        creationflags, start_new_session = self._process_creation_options()
        start = time.monotonic()
        proc = subprocess.Popen(
            command,
            shell=True,
            cwd=str(cwd),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="surrogateescape",
            creationflags=creationflags,
            start_new_session=start_new_session,
        )
        self._register_process(proc)
        try:
            stdout, stderr = proc.communicate(timeout=effective_timeout_ms / 1000)
            timed_out = False
        except subprocess.TimeoutExpired as exc:
            stdout = _coerce_output(exc.output)
            stderr = _coerce_output(exc.stderr)
            timed_out = True
            self._terminate_process(proc)
            timeout_message = f"Command timed out after {effective_timeout_ms} ms"
            stderr = self._append_timeout_message(stderr, timeout_message)
        except BaseException:
            self._terminate_process(proc)
            raise
        finally:
            self._unregister_process(proc)
        duration_ms = int(round((time.monotonic() - start) * 1000))
        exit_code = proc.returncode if proc.returncode is not None else 0
        if timed_out and exit_code == 0:
            exit_code = 124
        return ExecResult(
            stdout=stdout or "",
            stderr=stderr or "",
            exit_code=exit_code,
            timed_out=timed_out,
            duration_ms=duration_ms,
        )

    def _register_process(self, proc: subprocess.Popen[str]) -> None:
        should_terminate = False
        with self._process_lock:
            self._active_processes.add(proc)
            should_terminate = self._cleanup_started
        if should_terminate:
            self._terminate_process(proc)

    def _unregister_process(self, proc: subprocess.Popen[str]) -> None:
        with self._process_lock:
            self._active_processes.discard(proc)

    def _build_environment(self, env_vars: Mapping[str, str] | None) -> dict[str, str]:
        if self._environment_inheritance_policy == EnvironmentInheritancePolicy.INHERIT_ALL:
            env = dict(os.environ)
        elif self._environment_inheritance_policy == EnvironmentInheritancePolicy.INHERIT_NONE:
            env = {}
        else:
            env = {
                key: value
                for key, value in os.environ.items()
                if _is_core_env_key(key) and not _is_sensitive_env_key(key)
            }
        if env_vars:
            for key, value in env_vars.items():
                env[str(key)] = str(value)
        return env

    def _append_timeout_message(self, output: str, message: str) -> str:
        if not output:
            return message
        separator = "" if output.endswith("\n") else "\n"
        return f"{output}{separator}{message}"

    def _process_creation_options(self) -> tuple[int, bool]:
        if os.name == "nt":
            return subprocess.CREATE_NEW_PROCESS_GROUP, False
        return 0, True

    def _process_group_is_alive(self, process_group_id: int) -> bool:
        if not hasattr(os, "killpg"):
            return False
        try:
            os.killpg(process_group_id, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True

    def _terminate_process(self, proc: subprocess.Popen[str]) -> None:
        if os.name == "nt":
            if hasattr(signal, "CTRL_BREAK_EVENT"):
                try:
                    proc.send_signal(signal.CTRL_BREAK_EVENT)
                except (OSError, ValueError):
                    proc.terminate()
            else:
                proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
            return

        grace_period_seconds = 2
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        deadline = time.monotonic() + grace_period_seconds
        while proc.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                proc.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                break

        if self._process_group_is_alive(proc.pid):
            remaining = deadline - time.monotonic()
            if remaining > 0:
                time.sleep(remaining)

        if self._process_group_is_alive(proc.pid):
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        proc.wait()

    def grep(self, pattern: str, path: str | Path, options: GrepOptions) -> str:
        if options.max_results < 1:
            raise ValueError("max_results must be at least 1")
        search_root = self._resolve_path(path)
        if not search_root.exists():
            raise FileNotFoundError(search_root)
        search_target = search_root

        rg_path = shutil.which("rg")
        if rg_path is not None:
            return self._grep_with_ripgrep(search_target, pattern, options, rg_path)
        return self._grep_with_python(search_target, pattern, options)

    def _grep_with_ripgrep(
        self,
        search_target: Path,
        pattern: str,
        options: GrepOptions,
        rg_path: str,
    ) -> str:
        command = [
            rg_path,
            "--json",
            "--no-config",
        ]
        if options.case_insensitive:
            command.append("-i")
        if options.glob_filter:
            command.extend(["-g", options.glob_filter])
        command.extend(["-e", pattern, str(search_target)])
        completed = subprocess.run(
            command,
            cwd=str(self._resolved_working_directory),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="surrogateescape",
            check=False,
        )
        if completed.returncode == 1:
            return ""
        if completed.returncode != 0:
            raise ValueError(
                (completed.stderr or completed.stdout or "ripgrep search failed").strip()
            )
        matches: list[str] = []
        for line in completed.stdout.splitlines():
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get("type") != "match":
                continue
            data = record.get("data")
            if not isinstance(data, dict):
                continue
            path_info = data.get("path")
            lines_info = data.get("lines")
            line_number = data.get("line_number")
            if not isinstance(path_info, dict) or not isinstance(lines_info, dict):
                continue
            path_text = path_info.get("text")
            line_text = lines_info.get("text")
            if not isinstance(path_text, str) or not isinstance(line_text, str):
                continue
            if not isinstance(line_number, int):
                continue
            display_path = self._display_search_path(path_text)
            line_text = line_text.rstrip("\r\n")
            matches.append(f"{display_path}:{line_number}:{line_text}")
            if len(matches) >= options.max_results:
                break
        return "\n".join(matches)

    def _grep_with_python(self, search_target: Path, pattern: str, options: GrepOptions) -> str:
        flags = re.IGNORECASE if options.case_insensitive else 0
        try:
            compiled = re.compile(pattern, flags)
        except re.error as exc:
            raise ValueError(str(exc)) from exc

        candidates = self._iter_grep_files(search_target)
        matches: list[str] = []
        for file_path in candidates:
            relative_path = self._display_path(file_path)
            if options.glob_filter and not fnmatch.fnmatch(relative_path, options.glob_filter):
                continue
            text = file_path.read_text(encoding="utf-8", errors="surrogateescape")
            for line_number, line in enumerate(text.splitlines(), start=1):
                if compiled.search(line):
                    matches.append(f"{relative_path}:{line_number}:{line}")
                    if len(matches) >= options.max_results:
                        return "\n".join(matches)
        return "\n".join(matches)

    def _iter_grep_files(self, search_target: Path) -> list[Path]:
        if search_target.is_file():
            return [search_target]
        files = [path for path in sorted(search_target.rglob("*")) if path.is_file()]
        return files

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        base = self._resolve_path(path)
        if not base.exists():
            raise FileNotFoundError(base)
        if not base.is_dir():
            raise NotADirectoryError(base)
        try:
            matches = [candidate for candidate in base.glob(pattern) if candidate.is_file()]
        except (NotImplementedError, ValueError) as exc:
            # pathlib rejects absolute and otherwise unsupported glob patterns.
            raise ValueError(str(exc)) from exc
        matches.sort(
            key=lambda candidate: (
                -candidate.stat().st_mtime_ns,
                self._display_path(candidate),
            )
        )
        return [self._display_path(candidate) for candidate in matches]


__all__ = ["LocalExecutionEnvironment"]
