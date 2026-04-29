from __future__ import annotations

import os
import stat
import subprocess
from pathlib import Path


def _write_fake_docker(bin_dir: Path, log_path: Path) -> None:
    docker = bin_dir / "docker"
    docker.write_text(
        "\n".join(
            [
                "#!/usr/bin/env bash",
                "set -euo pipefail",
                'printf "args=%s\\n" "$*" > "$FAKE_DOCKER_LOG"',
                'printf "provider=%s\\n" "${SPARK_TEST_PROVIDER_VALUE:-}" >> "$FAKE_DOCKER_LOG"',
            ]
        )
        + "\n"
    )
    docker.chmod(docker.stat().st_mode | stat.S_IXUSR)


def _run_docker_script(
    *,
    tmp_path: Path,
    spark_home: Path,
    codex_home: Path,
    log_path: Path,
) -> subprocess.CompletedProcess[str]:
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    _write_fake_docker(fake_bin, log_path)

    env = {
        **os.environ,
        "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
        "HOME": str(tmp_path / "home"),
        "SPARK_DOCKER_HOME": str(spark_home),
        "CODEX_HOME": str(codex_home),
        "FAKE_DOCKER_LOG": str(log_path),
    }

    return subprocess.run(
        ["bash", "scripts/run-docker.sh"],
        check=True,
        capture_output=True,
        text=True,
        env=env,
    )


def test_run_docker_seeds_codex_auth_config_and_sources_provider_env(tmp_path: Path) -> None:
    spark_home = tmp_path / "spark-docker"
    codex_home = tmp_path / "codex"
    log_path = tmp_path / "docker.log"

    codex_home.mkdir()
    (codex_home / "auth.json").write_text("host-auth\n")
    (codex_home / "config.toml").write_text("host-config\n")
    provider_dir = spark_home / "config"
    provider_dir.mkdir(parents=True)
    (provider_dir / "provider.env").write_text("SPARK_TEST_PROVIDER_VALUE=from-provider-env\n")

    _run_docker_script(
        tmp_path=tmp_path,
        spark_home=spark_home,
        codex_home=codex_home,
        log_path=log_path,
    )

    docker_codex_home = spark_home / "runtime" / "codex" / ".codex"
    assert docker_codex_home.is_dir()
    assert (docker_codex_home / "auth.json").read_text() == "host-auth\n"
    assert (docker_codex_home / "config.toml").read_text() == "host-config\n"
    assert stat.S_IMODE((docker_codex_home / "auth.json").stat().st_mode) == 0o600
    assert stat.S_IMODE((docker_codex_home / "config.toml").stat().st_mode) == 0o600
    assert log_path.read_text() == (
        "args=compose -f compose.package.yaml up --build\n"
        "provider=from-provider-env\n"
    )


def test_run_docker_preserves_existing_docker_codex_files(tmp_path: Path) -> None:
    spark_home = tmp_path / "spark-docker"
    codex_home = tmp_path / "codex"
    log_path = tmp_path / "docker.log"
    docker_codex_home = spark_home / "runtime" / "codex" / ".codex"

    codex_home.mkdir()
    (codex_home / "auth.json").write_text("host-auth\n")
    (codex_home / "config.toml").write_text("host-config\n")
    docker_codex_home.mkdir(parents=True)
    (docker_codex_home / "auth.json").write_text("docker-auth\n")
    (docker_codex_home / "config.toml").write_text("docker-config\n")

    _run_docker_script(
        tmp_path=tmp_path,
        spark_home=spark_home,
        codex_home=codex_home,
        log_path=log_path,
    )

    assert (docker_codex_home / "auth.json").read_text() == "docker-auth\n"
    assert (docker_codex_home / "config.toml").read_text() == "docker-config\n"


def test_run_docker_tolerates_missing_host_codex_files(tmp_path: Path) -> None:
    spark_home = tmp_path / "spark-docker"
    codex_home = tmp_path / "codex"
    log_path = tmp_path / "docker.log"

    _run_docker_script(
        tmp_path=tmp_path,
        spark_home=spark_home,
        codex_home=codex_home,
        log_path=log_path,
    )

    docker_codex_home = spark_home / "runtime" / "codex" / ".codex"
    assert docker_codex_home.is_dir()
    assert not (docker_codex_home / "auth.json").exists()
    assert not (docker_codex_home / "config.toml").exists()
    assert "args=compose -f compose.package.yaml up --build\n" in log_path.read_text()
