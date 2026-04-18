from __future__ import annotations

from pathlib import Path

import pytest

import spark_common.codex_runtime as codex_runtime_module
from spark_common.codex_runtime import build_codex_runtime_environment


def test_build_codex_runtime_environment_isolates_home_and_seeds_runtime_state(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "codex-runtime"
    seed_dir = tmp_path / "codex-seed"
    seed_dir.mkdir()
    (seed_dir / "auth.json").write_text('{"token":"seed"}', encoding="utf-8")
    (seed_dir / "config.toml").write_text("model = 'gpt-test'\n", encoding="utf-8")
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(seed_dir))
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(runtime_root)
    assert env["CODEX_HOME"] == str(runtime_root / ".codex")
    assert env["XDG_CONFIG_HOME"] == str(runtime_root / ".config")
    assert env["XDG_DATA_HOME"] == str(runtime_root / ".local/share")
    assert (runtime_root / ".codex" / "auth.json").read_text(encoding="utf-8") == '{"token":"seed"}'
    assert (runtime_root / ".codex" / "config.toml").read_text(encoding="utf-8") == "model = 'gpt-test'\n"


def test_build_codex_runtime_environment_defaults_to_spark_home_runtime_root(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    host_home = tmp_path / "host-home"
    expected_runtime_root = host_home / ".spark" / "runtime" / "codex"
    monkeypatch.setenv("HOME", str(host_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("SPARK_HOME", raising=False)
    monkeypatch.delenv("ATTRACTOR_CODEX_RUNTIME_ROOT", raising=False)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(expected_runtime_root)
    assert env["CODEX_HOME"] == str(expected_runtime_root / ".codex")
    assert env["XDG_CONFIG_HOME"] == str(expected_runtime_root / ".config")
    assert env["XDG_DATA_HOME"] == str(expected_runtime_root / ".local/share")


def test_build_codex_runtime_environment_uses_spark_home_when_set(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    spark_home = tmp_path / "spark-home"
    expected_runtime_root = spark_home / "runtime" / "codex"
    monkeypatch.setenv("HOME", str(tmp_path / "host-home"))
    monkeypatch.setenv("SPARK_HOME", str(spark_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("ATTRACTOR_CODEX_RUNTIME_ROOT", raising=False)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(expected_runtime_root)
    assert env["CODEX_HOME"] == str(expected_runtime_root / ".codex")
    assert env["XDG_CONFIG_HOME"] == str(expected_runtime_root / ".config")
    assert env["XDG_DATA_HOME"] == str(expected_runtime_root / ".local/share")


def test_build_codex_runtime_environment_respects_explicit_codex_and_xdg_overrides(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    spark_home = tmp_path / "spark-home"
    runtime_root = spark_home / "runtime" / "codex"
    explicit_codex_home = tmp_path / "custom-codex-home"
    explicit_xdg_config_home = tmp_path / "custom-config"
    explicit_xdg_data_home = tmp_path / "custom-data"
    monkeypatch.setenv("SPARK_HOME", str(spark_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.setenv("CODEX_HOME", str(explicit_codex_home))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(explicit_xdg_config_home))
    monkeypatch.setenv("XDG_DATA_HOME", str(explicit_xdg_data_home))
    monkeypatch.delenv("ATTRACTOR_CODEX_RUNTIME_ROOT", raising=False)

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(runtime_root)
    assert env["CODEX_HOME"] == str(explicit_codex_home)
    assert env["XDG_CONFIG_HOME"] == str(explicit_xdg_config_home)
    assert env["XDG_DATA_HOME"] == str(explicit_xdg_data_home)
    assert explicit_codex_home.is_dir()
    assert explicit_xdg_config_home.is_dir()
    assert explicit_xdg_data_home.is_dir()


def test_build_codex_runtime_environment_falls_back_to_host_codex_home_when_seed_dir_missing(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "codex-runtime"
    host_home = tmp_path / "host-home"
    host_codex_home = host_home / ".codex"
    host_codex_home.mkdir(parents=True)
    (host_codex_home / "auth.json").write_text('{"token":"host-seed"}', encoding="utf-8")
    (host_codex_home / "config.toml").write_text("model = 'host-model'\n", encoding="utf-8")

    monkeypatch.setenv("HOME", str(host_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["CODEX_HOME"] == str(runtime_root / ".codex")
    assert (runtime_root / ".codex" / "auth.json").read_text(encoding="utf-8") == '{"token":"host-seed"}'
    assert (runtime_root / ".codex" / "config.toml").read_text(encoding="utf-8") == "model = 'host-model'\n"


def test_build_codex_runtime_environment_falls_back_to_temp_runtime_root_when_persistent_root_creation_fails(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    spark_home = tmp_path / "spark-home"
    persistent_runtime_root = spark_home / "runtime" / "codex"
    temp_root_base = tmp_path / "temp-root"
    fallback_runtime_root = temp_root_base / "spark-codex-runtime"
    host_home = tmp_path / "host-home"
    host_codex_home = host_home / ".codex"
    host_codex_home.mkdir(parents=True)
    (host_codex_home / "auth.json").write_text('{"token":"host-seed"}', encoding="utf-8")

    def _ensure_directory(path: Path) -> None:
        if path == persistent_runtime_root:
            raise OSError("permission denied")
        path.mkdir(parents=True, exist_ok=True)

    monkeypatch.setenv("HOME", str(host_home))
    monkeypatch.setenv("SPARK_HOME", str(spark_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("ATTRACTOR_CODEX_RUNTIME_ROOT", raising=False)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)
    monkeypatch.setattr(codex_runtime_module, "_ensure_directory", _ensure_directory)
    monkeypatch.setattr(codex_runtime_module.tempfile, "gettempdir", lambda: str(temp_root_base))

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(fallback_runtime_root)
    assert env["CODEX_HOME"] == str(fallback_runtime_root / ".codex")
    assert env["XDG_CONFIG_HOME"] == str(fallback_runtime_root / ".config")
    assert env["XDG_DATA_HOME"] == str(fallback_runtime_root / ".local/share")
    assert (fallback_runtime_root / ".codex" / "auth.json").read_text(encoding="utf-8") == '{"token":"host-seed"}'


def test_build_codex_runtime_environment_reuses_same_persistent_runtime_root_across_calls(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    spark_home = tmp_path / "spark-home"
    expected_runtime_root = spark_home / "runtime" / "codex"
    monkeypatch.setenv("SPARK_HOME", str(spark_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("ATTRACTOR_CODEX_RUNTIME_ROOT", raising=False)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    first_env = build_codex_runtime_environment()
    second_env = build_codex_runtime_environment()

    assert first_env["HOME"] == str(expected_runtime_root)
    assert second_env["HOME"] == str(expected_runtime_root)
    assert first_env["CODEX_HOME"] == second_env["CODEX_HOME"] == str(expected_runtime_root / ".codex")


def test_build_codex_runtime_environment_prepends_first_party_tool_bins_to_path(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "codex-runtime"
    runtime_python = tmp_path / "runtime-python" / "bin" / "python"
    repo_root = tmp_path / "repo-root"
    repo_bin = repo_root / ".venv" / "bin"

    runtime_python.parent.mkdir(parents=True)
    runtime_python.write_text("", encoding="utf-8")
    repo_bin.mkdir(parents=True)

    monkeypatch.setattr(codex_runtime_module, "RUNTIME_REPO_ROOT", repo_root)
    monkeypatch.setattr(codex_runtime_module.sys, "executable", str(runtime_python))
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.setenv("PATH", os_path := f"/usr/local/bin{codex_runtime_module.os.pathsep}/usr/bin")
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["PATH"] == codex_runtime_module.os.pathsep.join(
        [
            str(runtime_python.parent.resolve(strict=False)),
            str(repo_bin.resolve(strict=False)),
            os_path,
        ]
    )
