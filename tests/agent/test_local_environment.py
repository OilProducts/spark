from __future__ import annotations

import os
import shlex
import shutil
import stat
import subprocess
import sys
import time
from pathlib import Path

import pytest

import agent


def _shell_command(*args: str) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(list(args))
    return shlex.join(args)


def _python_command(code: str) -> str:
    return _shell_command(sys.executable, "-c", code)


def test_local_environment_resolves_paths_and_handles_lifecycle(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)

    assert environment.working_directory() == str(workspace)
    assert workspace.exists() is False

    environment.initialize()
    assert workspace.exists() is True
    assert workspace.is_dir() is True
    assert environment.platform() in {"darwin", "linux", "windows", "wasm"}
    assert isinstance(environment.os_version(), str)
    assert environment.os_version()

    environment.write_file("nested/example.txt", "first line\nsecond line\n")
    assert environment.file_exists("nested/example.txt") is True
    assert environment.is_directory("nested") is True
    assert environment.read_file("nested/example.txt") == "first line\nsecond line\n"
    assert environment.read_file("nested/example.txt", offset=2, limit=1) == "second line\n"
    environment.write_file("nested/rename_me.txt", "rename me\n")
    environment.rename_file("nested/rename_me.txt", "nested/renamed.txt")
    assert environment.file_exists("nested/rename_me.txt") is False
    assert environment.read_file("nested/renamed.txt") == "rename me\n"
    environment.delete_file("nested/renamed.txt")
    assert environment.file_exists("nested/renamed.txt") is False

    entries = environment.list_directory(".", depth=1)
    assert entries == [
        agent.DirEntry(name="nested", is_dir=True, size=None),
        agent.DirEntry(name="nested/example.txt", is_dir=False, size=23),
    ]

    environment.cleanup()


def test_local_environment_exec_command_uses_requested_working_directory(
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    subdirectory = workspace / "subdir"
    subdirectory.mkdir(parents=True)
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)

    result = environment.exec_command(
        _python_command("import os; print(os.getcwd())"),
        working_dir="subdir",
        timeout_ms=1000,
    )

    assert result.exit_code == 0
    assert result.timed_out is False
    assert result.stdout.strip() == str(subdirectory.resolve())
    assert result.stderr == ""
    assert result.duration_ms >= 0


def test_local_environment_exec_command_uses_platform_default_shell(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)

    if os.name == "nt":
        result = environment.exec_command("echo %COMSPEC%", timeout_ms=1000)
        assert result.exit_code == 0
        assert result.stdout.strip().lower().endswith("cmd.exe")
    else:
        result = environment.exec_command("echo $0", timeout_ms=1000)
        assert result.exit_code == 0
        shell_name = result.stdout.strip().lower()
        assert shell_name.endswith("sh")
        assert "bash" not in shell_name


def test_local_environment_exec_command_separates_stdout_stderr_and_exit_code(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)

    result = environment.exec_command(
        _python_command(
            "import sys; "
            "sys.stdout.write('out\\n'); "
            "sys.stderr.write('err\\n'); "
            "raise SystemExit(7)"
        ),
        timeout_ms=1000,
    )

    assert result.stdout == "out\n"
    assert result.stderr == "err\n"
    assert result.exit_code == 7
    assert result.timed_out is False


def test_local_environment_exec_command_times_out_and_returns_partial_output(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        default_command_timeout_ms=50,
    )

    result = environment.exec_command(
        _python_command("import time; print('start', flush=True); time.sleep(5)")
    )

    assert result.timed_out is True
    assert result.stdout == "start\n"
    assert result.stderr == "Command timed out after 50 ms"
    assert result.exit_code != 0


def test_local_environment_exec_command_caps_timeout_at_maximum(
    tmp_path: Path,
) -> None:
    environment = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        max_command_timeout_ms=50,
    )

    result = environment.exec_command(
        _python_command("import time; time.sleep(5)"),
        timeout_ms=5000,
    )

    assert result.timed_out is True
    assert result.stderr == "Command timed out after 50 ms"
    assert result.duration_ms < 1000


def test_local_environment_exec_command_terminates_process_group_on_timeout(
    tmp_path: Path,
) -> None:
    if os.name == "nt" or not hasattr(os, "killpg"):
        pytest.skip("POSIX signal handling is not available on Windows")

    marker = tmp_path / "terminated.txt"
    environment = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        default_command_timeout_ms=50,
    )

    child_code = (
        "import os, pathlib, signal, sys, time\n"
        "def handler(*_):\n"
        "    pathlib.Path(os.environ['MARKER']).write_text('terminated', encoding='utf-8')\n"
        "    sys.exit(0)\n"
        "signal.signal(signal.SIGTERM, handler)\n"
        "time.sleep(5)\n"
    )
    command = _python_command(
        "import subprocess, sys, time; "
        f"subprocess.Popen([sys.executable, '-c', {child_code!r}]); "
        "time.sleep(5)"
    )

    result = environment.exec_command(
        command,
        env_vars={"MARKER": str(marker)},
    )

    assert result.timed_out is True
    deadline = time.monotonic() + 3.5
    while time.monotonic() < deadline and not marker.exists():
        time.sleep(0.05)

    assert marker.read_text(encoding="utf-8") == "terminated"
    assert result.stderr == "Command timed out after 50 ms"


def test_local_environment_exec_command_kills_ignoring_child_after_timeout_grace_period(
    tmp_path: Path,
) -> None:
    if os.name == "nt" or not hasattr(os, "killpg"):
        pytest.skip("POSIX signal handling is not available on Windows")

    marker = tmp_path / "survived.txt"
    environment = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        default_command_timeout_ms=50,
    )
    child_script = (
        "import os, pathlib, signal, time; "
        "signal.signal(signal.SIGHUP, signal.SIG_IGN); "
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "time.sleep(3); "
        "pathlib.Path(os.environ['MARKER']).write_text('survived', encoding='utf-8')"
    )
    command = _python_command(
        "import subprocess, sys, time; "
        f"subprocess.Popen([sys.executable, '-c', {child_script!r}]); "
        "time.sleep(5)"
    )

    result = environment.exec_command(command, env_vars={"MARKER": str(marker)})

    assert result.timed_out is True
    assert result.stderr == "Command timed out after 50 ms"
    assert result.duration_ms >= 1900

    deadline = time.monotonic() + 3.5
    while time.monotonic() < deadline and not marker.exists():
        time.sleep(0.05)

    assert marker.exists() is False


def test_local_environment_environment_inheritance_policy_filters_sensitive_vars(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("Unit_Test_PaSsWoRd", "shh")
    monkeypatch.setenv("UNIT_TEST_VISIBLE", "visible")
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)

    command = _python_command(
        "import os; "
        "print(os.environ.get('Unit_Test_PaSsWoRd', 'unset')); "
        "print(os.environ.get('UNIT_TEST_VISIBLE', 'unset')); "
        "print(os.environ.get('EXPLICIT_VALUE', 'unset')); "
        "print(os.environ.get('PATH', 'unset'))"
    )
    default_result = environment.exec_command(
        command,
        timeout_ms=1000,
        env_vars={"EXPLICIT_VALUE": "from-env-vars"},
    )
    default_lines = default_result.stdout.splitlines()
    assert len(default_lines) == 4
    assert default_lines[:3] == [
        "unset",
        "unset",
        "from-env-vars",
    ]
    assert default_lines[3] != "unset"

    inherit_all = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        environment_inheritance_policy=agent.EnvironmentInheritancePolicy.INHERIT_ALL,
    )
    inherit_all_result = inherit_all.exec_command(
        command,
        timeout_ms=1000,
        env_vars={"EXPLICIT_VALUE": "from-env-vars"},
    )
    inherit_all_lines = inherit_all_result.stdout.splitlines()
    assert len(inherit_all_lines) == 4
    assert inherit_all_lines[:3] == [
        "shh",
        "visible",
        "from-env-vars",
    ]
    assert inherit_all_lines[3] != "unset"

    inherit_none = agent.LocalExecutionEnvironment(
        working_dir=tmp_path,
        environment_inheritance_policy=agent.EnvironmentInheritancePolicy.INHERIT_NONE,
    )
    inherit_none_result = inherit_none.exec_command(
        command,
        timeout_ms=1000,
        env_vars={"EXPLICIT_VALUE": "from-env-vars"},
    )
    inherit_none_lines = inherit_none_result.stdout.splitlines()
    assert len(inherit_none_lines) == 4
    assert inherit_none_lines[:3] == [
        "unset",
        "unset",
        "from-env-vars",
    ]


def test_local_environment_grep_uses_ripgrep_branch_when_available(
    tmp_path: Path,
    monkeypatch,
) -> None:
    workspace = tmp_path / "workspace"
    matched_file = workspace / "nested" / "sentinel.txt"
    matched_file.parent.mkdir(parents=True)
    matched_file.write_text("from-rg\n", encoding="utf-8")
    match_payload = {
        "type": "match",
        "data": {
            "path": {"text": str(matched_file.resolve())},
            "line_number": 1,
            "lines": {"text": "from-rg\n"},
        },
    }

    fake_rg = tmp_path / "rg"
    fake_rg.write_text(
        "#!/usr/bin/env python3\n"
        "import json\n"
        f"print(json.dumps({match_payload!r}))\n",
        encoding="utf-8",
    )
    fake_rg.chmod(fake_rg.stat().st_mode | stat.S_IEXEC)

    original_which = shutil.which

    def fake_which(name: str) -> str | None:
        if name == "rg":
            return str(fake_rg)
        return original_which(name)

    monkeypatch.setattr(shutil, "which", fake_which)
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)

    result = environment.grep("pattern", ".", agent.GrepOptions(max_results=1))

    assert result == "nested/sentinel.txt:1:from-rg"


def test_local_environment_grep_falls_back_to_python_regex_with_filters_and_case_insensitive(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setattr(shutil, "which", lambda name: None)
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file(
        "notes.txt",
        "Alpha\nignored\nALPHA\n",
    )
    environment.write_file("nested/ignore.md", "alpha\n")

    result = environment.grep(
        "alpha",
        ".",
        agent.GrepOptions(
            glob_filter="*.txt",
            case_insensitive=True,
            max_results=2,
        ),
    )

    assert result.splitlines() == ["notes.txt:1:Alpha", "notes.txt:3:ALPHA"]


def test_local_environment_grep_reports_invalid_regex_and_missing_path(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setattr(shutil, "which", lambda name: None)
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    environment.write_file("notes.txt", "alpha\n")

    with pytest.raises(ValueError):
        environment.grep("(", ".", agent.GrepOptions())

    with pytest.raises(FileNotFoundError):
        environment.grep("alpha", "missing", agent.GrepOptions())


def test_local_environment_glob_returns_newest_files_first(tmp_path: Path) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    old_file = tmp_path / "alpha.txt"
    new_file = tmp_path / "omega.txt"
    old_file.write_text("old", encoding="utf-8")
    new_file.write_text("new", encoding="utf-8")

    now = time.time()
    os.utime(old_file, (now - 20, now - 20))
    os.utime(new_file, (now - 5, now - 5))

    assert environment.glob("*.txt", ".") == ["omega.txt", "alpha.txt"]
