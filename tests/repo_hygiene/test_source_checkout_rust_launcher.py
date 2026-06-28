from __future__ import annotations

import importlib.abc
import os
from pathlib import Path
import sys

import pytest

import spark._rust_launcher as launcher


def test_source_checkout_launcher_dispatches_to_workspace_rust_binary(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if os.name != "posix":
        pytest.skip("execv dispatch is only used on POSIX")
    workspace = _spark_workspace(tmp_path)
    rust_binary = _write_executable(workspace / "target" / "debug" / "spark")
    _point_launcher_at_workspace(monkeypatch, workspace)
    monkeypatch.setattr(sys, "argv", ["spark", "--help"])

    class ExecCalled(Exception):
        def __init__(self, path: str, argv: list[str]) -> None:
            super().__init__(path)
            self.path = path
            self.argv = argv

    def capture_execv(path: str, argv: list[str]) -> None:
        raise ExecCalled(path, argv)

    monkeypatch.setattr(launcher.os, "execv", capture_execv)

    with pytest.raises(ExecCalled) as captured:
        launcher.spark_main()

    assert captured.value.path == str(rust_binary)
    assert captured.value.argv == [str(rust_binary), "--help"]


def test_source_checkout_launcher_missing_binary_fails_with_rust_build_instructions(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    workspace = _spark_workspace(tmp_path)
    _point_launcher_at_workspace(monkeypatch, workspace)
    monkeypatch.setattr(sys, "argv", ["spark-server", "--help"])
    _block_python_runtime_fallback_imports(monkeypatch)

    exit_code = launcher.spark_server_main()

    assert exit_code == launcher.EXIT_RUST_BINARY_UNAVAILABLE
    stderr = capsys.readouterr().err
    assert "could not find the Rust `spark-server` binary" in stderr
    assert "does not fall back to the Python Spark CLI, server, or provider runtime" in stderr
    assert "cargo build -p spark-server --bin spark-server" in stderr
    assert "uv run spark-server --help" in stderr


def _spark_workspace(tmp_path: Path) -> Path:
    workspace = tmp_path / "workspace"
    workspace.joinpath("src", "spark").mkdir(parents=True)
    workspace.joinpath("crates", "spark-cli").mkdir(parents=True)
    workspace.joinpath("crates", "spark-server").mkdir(parents=True)
    workspace.joinpath("Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/spark-cli", "crates/spark-server"]\n',
        encoding="utf-8",
    )
    workspace.joinpath("pyproject.toml").write_text("[project]\nname = \"spark\"\n", encoding="utf-8")
    workspace.joinpath("crates", "spark-cli", "Cargo.toml").write_text(
        "[package]\nname = \"spark-cli\"\n",
        encoding="utf-8",
    )
    workspace.joinpath("crates", "spark-server", "Cargo.toml").write_text(
        "[package]\nname = \"spark-server\"\n",
        encoding="utf-8",
    )
    return workspace


def _point_launcher_at_workspace(monkeypatch: pytest.MonkeyPatch, workspace: Path) -> None:
    monkeypatch.setattr(
        launcher,
        "__file__",
        str(workspace / "src" / "spark" / "_rust_launcher.py"),
    )


def _write_executable(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    path.chmod(0o755)
    return path


def _block_python_runtime_fallback_imports(monkeypatch: pytest.MonkeyPatch) -> None:
    blocked = {
        "spark.cli",
        "spark.server_cli",
        "unified_llm",
        "unified_llm.adapters",
        "unified_llm.provider_utils",
    }
    for module_name in blocked:
        monkeypatch.delitem(sys.modules, module_name, raising=False)

    class BlockedImportFinder(importlib.abc.MetaPathFinder):
        def find_spec(self, fullname: str, path: object, target: object = None) -> object:
            if fullname in blocked:
                raise AssertionError(f"unexpected Python runtime fallback import: {fullname}")
            return None

    monkeypatch.setattr(sys, "meta_path", [BlockedImportFinder(), *sys.meta_path])
