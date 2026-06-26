from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import importlib.resources
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import sysconfig
import tarfile
import time
from typing import Any, Iterator, Mapping
import venv
import zipfile

import httpx
import pytest

from tests.compat import harness
from tests.compat.conftest import (
    CompatSourceUi,
    ITEM_DECISIONS,
    ITEM_ID_M0_I05,
    ITEM_REQUIREMENTS,
)


@dataclass(frozen=True)
class BuildSmokeObservation:
    manifest: dict[str, Any]
    wheel_path: Path | None


M6_DOCKER_PROVIDER_ENV_KEYS = (
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "OPENAI_COMPATIBLE_BASE_URL",
    "OPENAI_COMPATIBLE_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "GEMINI_API_KEY",
    "GEMINI_BASE_URL",
    "GOOGLE_API_KEY",
    "OPENROUTER_API_KEY",
    "OPENROUTER_BASE_URL",
    "OPENROUTER_HTTP_REFERER",
    "OPENROUTER_TITLE",
    "LITELLM_BASE_URL",
    "LITELLM_API_KEY",
)

def test_source_ui_resolution_smoke_fixture_matches_python_oracle(
    tmp_path: Path,
    rewrite_worktree_path: Path,
    compat_source_ui_dir: CompatSourceUi,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    with _running_source_ui_server(
        tmp_path=tmp_path,
        rewrite_worktree_path=rewrite_worktree_path,
        source_ui=compat_source_ui_dir,
    ) as server:
        requests = []
        with httpx.Client(base_url=server["base_url"], timeout=10.0) as client:
            for path in (
                "/",
                "/assets/compat-app.js",
                "/favicon.ico",
                "/assets/missing.png",
                "/workspace/api/missing",
            ):
                response = client.get(path)
                requests.append(
                    {
                        "request": {"method": "GET", "path": path},
                        "response": {
                            "status_code": response.status_code,
                            "headers": harness.selected_http_headers(response.headers),
                            "body": harness.http_body_observation(
                                content=response.content,
                                headers=response.headers,
                            ),
                        },
                    }
                )
    manifest = _packaging_manifest(
        fixture_id="packaging/source-ui-resolution-smoke",
        scenario="source_ui_resolution_smoke",
        provenance_interfaces=[
            "spark-server serve --ui-dir",
            "GET /",
            "GET /assets/{asset_path}",
            "GET /favicon.ico",
            "GET /workspace/api/missing",
        ],
        server={
            "base_url": server["base_url"],
            "spark_home": str(server["spark_home"]),
            "flows_dir": str(server["flows_dir"]),
            "ui_dir": str(compat_source_ui_dir.root),
        },
        requests=requests,
    )
    manifest = harness.normalize_path_tokens(
        manifest,
        {
            "__SPARK_HOME__": server["spark_home"],
            "__SPARK_FLOWS_DIR__": server["flows_dir"],
            "__SPARK_UI_DIR__": compat_source_ui_dir.root,
        },
    )
    _assert_packaging_fixture(
        manifest,
        compat_fixture_root / "packaging/source-ui-resolution-smoke.json",
        compat_update_goldens,
    )


def test_build_deliverable_smoke_fixture_matches_python_oracle(
    packaging_build_smoke: BuildSmokeObservation,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    _assert_packaging_fixture(
        packaging_build_smoke.manifest,
        compat_fixture_root / "packaging/build-deliverable-smoke.json",
        compat_update_goldens,
    )


def test_installed_command_init_smoke_fixture_matches_python_oracle(
    tmp_path: Path,
    packaging_build_smoke: BuildSmokeObservation,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    manifest = _installed_command_manifest(
        tmp_path=tmp_path,
        wheel_path=packaging_build_smoke.wheel_path,
    )
    _assert_packaging_fixture(
        manifest,
        compat_fixture_root / "packaging/installed-command-init-smoke.json",
        compat_update_goldens,
    )


def test_service_workflow_smoke_fixture_matches_python_oracle(
    tmp_path: Path,
    rewrite_worktree_path: Path,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    data_dir = tmp_path / "service-home"
    flows_dir = tmp_path / "service-flows"
    ui_dir = tmp_path / "service-ui"
    xdg_config_home = tmp_path / "xdg-config"
    fake_systemctl_dir = tmp_path / "fake-bin"
    fake_systemctl_log = tmp_path / "systemctl-calls.jsonl"
    ui_dir.mkdir(parents=True)
    (ui_dir / "index.html").write_text("<!doctype html><html></html>\n", encoding="utf-8")
    _write_fake_systemctl(fake_systemctl_dir / "systemctl")
    env = {
        **os.environ,
        "PATH": os.pathsep.join([str(fake_systemctl_dir), os.environ.get("PATH", "")]),
        "XDG_CONFIG_HOME": str(xdg_config_home),
        "SPARK_HOME": str(data_dir),
        "SPARK_FLOWS_DIR": str(flows_dir),
        "SPARK_UI_DIR": str(ui_dir),
        "SPARK_COMPAT_SYSTEMCTL_LOG": str(fake_systemctl_log),
    }
    service_unit_path = xdg_config_home / "systemd" / "user" / "spark.service"
    install = _run_process(
        [
            sys.executable,
            "-m",
            "spark.server_cli",
            "service",
            "install",
            "--host",
            "127.0.0.1",
            "--port",
            "8123",
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
            "--ui-dir",
            str(ui_dir),
        ],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=60,
    )
    unit_text = service_unit_path.read_text(encoding="utf-8") if service_unit_path.exists() else ""
    seeded_flows = sorted(
        path.relative_to(flows_dir).as_posix()
        for path in flows_dir.rglob("*.dot")
    ) if flows_dir.exists() else []
    status = _run_process(
        [
            sys.executable,
            "-m",
            "spark.server_cli",
            "service",
            "status",
        ],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=30,
    )
    remove = _run_process(
        [
            sys.executable,
            "-m",
            "spark.server_cli",
            "service",
            "remove",
        ],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=30,
    )
    manifest = _packaging_manifest(
        fixture_id="packaging/service-workflow-smoke",
        scenario="service_workflow_smoke",
        provenance_interfaces=[
            "spark-server service install",
            "spark-server service status",
            "spark-server service remove",
            "systemctl --user",
        ],
        command={
            "argv": install["argv"],
            "cwd": install["cwd"],
        },
        process=install,
        observations={
            "install": _process_excerpt(install),
            "status": _process_excerpt(status),
            "remove": _process_excerpt(remove),
            "service_unit_path": str(service_unit_path),
            "unit_after_install_exists": bool(unit_text),
            "unit_after_remove_exists": service_unit_path.exists(),
            "unit_sections": _service_unit_observation(unit_text),
            "systemctl_invocations": _read_fake_systemctl_invocations(fake_systemctl_log),
            "seeded_flows": seeded_flows,
        },
    )
    manifest = harness.normalize_path_tokens(
        manifest,
        {
            "__WORKTREE__": rewrite_worktree_path,
            "__SPARK_HOME__": data_dir,
            "__SPARK_FLOWS_DIR__": flows_dir,
            "__SPARK_UI_DIR__": ui_dir,
            "__XDG_CONFIG_HOME__": xdg_config_home,
            "__FAKE_SYSTEMCTL_DIR__": fake_systemctl_dir,
            "__SYSTEMCTL_LOG__": fake_systemctl_log,
        },
    )
    _assert_packaging_fixture(
        manifest,
        compat_fixture_root / "packaging/service-workflow-smoke.json",
        compat_update_goldens,
    )


def test_package_resource_presence_fixture_matches_python_oracle(
    packaging_build_smoke: BuildSmokeObservation,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    manifest = _packaging_manifest(
        fixture_id="packaging/package-resource-presence",
        scenario="package_resource_presence",
        provenance_interfaces=[
            "importlib.resources package resource lookup",
            "built wheel content inspection when deliverable smoke succeeds",
        ],
        resources={
            "source_package": _source_package_resources(),
            "built_wheel": _wheel_resource_presence(packaging_build_smoke.wheel_path),
        },
    )
    _assert_packaging_fixture(
        manifest,
        compat_fixture_root / "packaging/package-resource-presence.json",
        compat_update_goldens,
    )


def test_m6_deliverable_contains_rust_binaries_and_excludes_generated_paths(
    packaging_build_smoke: BuildSmokeObservation,
) -> None:
    wheel_path = packaging_build_smoke.wheel_path
    if wheel_path is None:
        pytest.skip("build deliverable smoke did not produce a wheel")
    sdist_paths = sorted(wheel_path.parent.glob("spark-*.tar.gz"))
    assert len(sdist_paths) == 1

    with zipfile.ZipFile(wheel_path) as wheel:
        names = set(wheel.namelist())
        info_by_name = {info.filename: info for info in wheel.infolist()}
        wheel_metadata = _wheel_metadata(wheel)
    required = {
        "spark/bin/spark",
        "spark/bin/spark-server",
        "spark/ui_dist/index.html",
        "spark/guides/dot-authoring.md",
        "spark/guides/spark-operations.md",
        "spark/flows/examples/simple-linear.dot",
        "spark/flows/software-development/spec-implementation/implement-spec.dot",
        "unified_llm/data/models.json",
    }
    assert required <= names
    assert any(name.startswith("spark/ui_dist/assets/") and name.endswith(".js") for name in names)
    assert any(name.startswith("spark/ui_dist/assets/") and name.endswith(".css") for name in names)
    native_tag = _native_wheel_tag()
    assert wheel_path.name.endswith(f"-{native_tag}.whl")
    assert wheel_metadata["root_is_purelib"] == "false"
    assert native_tag in wheel_metadata["tags"]
    assert "py3-none-any" not in wheel_metadata["tags"]
    for entry in ("spark/bin/spark", "spark/bin/spark-server"):
        assert (info_by_name[entry].external_attr >> 16) & 0o111
        with zipfile.ZipFile(wheel_path) as wheel:
            assert _is_native_binary_payload(wheel.read(entry))
    assert not _artifact_forbidden_entries(names)

    with tarfile.open(sdist_paths[0]) as sdist:
        sdist_names = set(sdist.getnames())
    sdist_relative_names = _sdist_relative_names(sdist_names)
    required_sdist_sources = {
        "Cargo.toml",
        "Cargo.lock",
        "Dockerfile",
        "pyproject.toml",
        "Dockerfile.wheel",
        "compose.package.yaml",
        "scripts/build_deliverable.py",
        "scripts/package-entrypoint.sh",
        "scripts/run-docker.sh",
        "crates/spark-cli/src/main.rs",
        "crates/spark-server/src/main.rs",
        "crates/spark-assets/build.rs",
        "crates/spark-assets/src/lib.rs",
        "crates/spark-workspace/src/models.rs",
        "crates/unified-llm-adapter/src/env.rs",
        "crates/unified-llm-adapter/src/catalog.rs",
        "crates/unified-llm-adapter/src/profiles.rs",
        "frontend/package.json",
        "frontend/package-lock.json",
        "frontend/src/main.tsx",
        "frontend/vite.config.ts",
        "src/spark/flows/examples/simple-linear.dot",
        "src/spark/guides/dot-authoring.md",
        "src/unified_llm/data/models.json",
        "tests/compat/providers/test_unified_llm_adapter_fixtures.py",
        "tests/compat/packaging/test_packaging_smoke_fixtures.py",
        "assets/spark-app-icon-dark.png",
    }
    assert required_sdist_sources <= sdist_relative_names
    assert "spark/bin/spark" not in sdist_relative_names
    assert "spark/bin/spark-server" not in sdist_relative_names
    assert "src/spark/bin/spark" not in sdist_relative_names
    assert "src/spark/bin/spark-server" not in sdist_relative_names
    assert not _artifact_forbidden_entries(sdist_names)


def test_m6_installed_commands_dispatch_to_packaged_rust_binaries(
    tmp_path: Path,
    packaging_build_smoke: BuildSmokeObservation,
) -> None:
    wheel_path = packaging_build_smoke.wheel_path
    if wheel_path is None:
        pytest.skip("build deliverable smoke did not produce a wheel")

    venv_dir = tmp_path / "rust-installed-venv"
    venv.EnvBuilder(with_pip=True, system_site_packages=True).create(venv_dir)
    python_bin = _venv_bin(venv_dir, "python")
    install = _run_process(
        [str(python_bin), "-m", "pip", "install", str(wheel_path)],
        cwd=tmp_path,
        timeout=120,
    )
    assert install["returncode"] == 0, install["stderr"]

    binary_paths = _installed_rust_binary_paths(python_bin)
    for command_name, binary_path in binary_paths.items():
        assert binary_path.is_file()
        assert os.access(binary_path, os.X_OK)
        help_result = _run_process([str(binary_path), "--help"], cwd=tmp_path, timeout=30)
        assert help_result["returncode"] == 0
        assert command_name in help_result["stdout"]

    home_root = tmp_path / "home"
    home_root.mkdir()
    init_env = {
        key: value
        for key, value in os.environ.items()
        if key not in {"SPARK_HOME", "SPARK_FLOWS_DIR"}
    }
    init_env["HOME"] = str(home_root)
    default_init = _run_process(
        [str(binary_paths["spark-server"]), "init"],
        cwd=tmp_path,
        env=init_env,
        timeout=60,
    )
    assert default_init["returncode"] == 0, default_init["stderr"]
    assert "Initialized Spark at" in default_init["stdout"]
    assert (home_root / ".spark" / "flows" / "examples" / "simple-linear.dot").is_file()


def test_m6_installed_init_serve_and_service_workflows(
    tmp_path: Path,
    packaging_build_smoke: BuildSmokeObservation,
) -> None:
    wheel_path = packaging_build_smoke.wheel_path
    if wheel_path is None:
        pytest.skip("build deliverable smoke did not produce a wheel")

    venv_dir = tmp_path / "installed-workflow-venv"
    venv.EnvBuilder(with_pip=True, system_site_packages=True).create(venv_dir)
    python_bin = _venv_bin(venv_dir, "python")
    spark_server_bin = _venv_bin(venv_dir, "spark-server")
    install = _run_process(
        [str(python_bin), "-m", "pip", "install", str(wheel_path)],
        cwd=tmp_path,
        timeout=120,
    )
    assert install["returncode"] == 0, install["stderr"]
    rust_binary_paths = _installed_rust_binary_paths(python_bin)

    home_root = tmp_path / "installed-home"
    home_root.mkdir()
    default_env = _without_spark_path_env(os.environ)
    default_env["HOME"] = str(home_root)
    default_init = _run_process(
        [str(spark_server_bin), "init"],
        cwd=tmp_path,
        env=default_env,
        timeout=60,
    )
    assert default_init["returncode"] == 0, default_init["stderr"]
    default_home = home_root / ".spark"
    assert (default_home / "config/flow-catalog.toml").is_file()
    assert len(list(default_home.joinpath("flows").rglob("*.dot"))) == 9

    data_dir = tmp_path / "workflow-home"
    flows_dir = tmp_path / "workflow-flows"
    explicit_init = _run_process(
        [
            str(spark_server_bin),
            "init",
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=tmp_path,
        env=default_env,
        timeout=60,
    )
    assert explicit_init["returncode"] == 0, explicit_init["stderr"]
    assert "created=9 updated=0 skipped=0" in explicit_init["stdout"]
    edited_flow = flows_dir / "examples/simple-linear.dot"
    edited_flow.write_text("digraph UserEdited {}\n", encoding="utf-8")
    skipped_init = _run_process(
        [
            str(spark_server_bin),
            "init",
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=tmp_path,
        env=default_env,
        timeout=60,
    )
    assert skipped_init["returncode"] == 0, skipped_init["stderr"]
    assert "created=0 updated=0 skipped=9" in skipped_init["stdout"]
    assert edited_flow.read_text(encoding="utf-8") == "digraph UserEdited {}\n"
    forced_init = _run_process(
        [
            str(spark_server_bin),
            "init",
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
            "--force",
        ],
        cwd=tmp_path,
        env=default_env,
        timeout=60,
    )
    assert forced_init["returncode"] == 0, forced_init["stderr"]
    assert "created=0 updated=9 skipped=0" in forced_init["stdout"]
    assert edited_flow.read_text(encoding="utf-8") != "digraph UserEdited {}\n"

    profile_key = "SPARK_COMPAT_PROFILE_KEY"
    default_env.pop(profile_key, None)
    profiles_path = data_dir / "config/llm-profiles.toml"
    profiles_path.write_text(
        """
[profiles.local]
label = "Installed Local"
provider = "openai_compatible"
base_url = "http://127.0.0.1:65535/v1"
api_key_env = "SPARK_COMPAT_PROFILE_KEY"
models = ["installed-local"]
default_model = "installed-local"
""".strip(),
        encoding="utf-8",
    )
    provider_env = data_dir / "config/provider.env"
    provider_env.write_text("SPARK_COMPAT_PROFILE_KEY=from-provider-env\n", encoding="utf-8")

    port = _free_tcp_port()
    server_env = {**default_env, "SPARK_HOME": str(data_dir), "SPARK_FLOWS_DIR": str(flows_dir)}
    stdout = (tmp_path / "installed-serve.stdout.log").open("w", encoding="utf-8")
    stderr = (tmp_path / "installed-serve.stderr.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(spark_server_bin),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=tmp_path,
        env=server_env,
        text=True,
        stdout=stdout,
        stderr=stderr,
    )
    base_url = f"http://127.0.0.1:{port}"
    try:
        _wait_for_server(base_url, process)
        with httpx.Client(base_url=base_url, timeout=10.0) as client:
            index = client.get("/")
            assert index.status_code == 200
            assert index.headers["content-type"].startswith("text/html")
            assert "<!doctype html" in index.text.lower()
            favicon = client.get("/favicon.ico")
            assert favicon.status_code == 200
            assert favicon.headers["content-type"] == "image/png"
            assert favicon.content
            settings = client.get("/workspace/api/settings")
            assert settings.status_code == 200
            assert settings.headers["content-type"] == "application/json"
            assert settings.json()["execution_placement"]["profiles"][0]["id"] == "native"
            attractor_status = client.get("/attractor/status")
            assert attractor_status.status_code == 200
            assert attractor_status.json()["status"] == "idle"
            chat_models = client.get(
                "/workspace/api/projects/chat-models",
                params={"project_path": str(tmp_path)},
            )
            assert chat_models.status_code == 200
            chat_payload = chat_models.json()
            assert any(
                model["provider"] == "openai" and model["id"] == "gpt-5.2"
                for model in chat_payload["models"]
            )
            assert any(
                model["provider"] == "openai_compatible"
                and model["id"] == "installed-local"
                and model["display"] == "Installed Local / installed-local"
                for model in chat_payload["models"]
            )
            assert "127.0.0.1:65535" not in chat_models.text
            assert profile_key not in chat_models.text
            profiles = client.get("/attractor/api/llm-profiles")
            assert profiles.status_code == 200
            profile_payload = profiles.json()
            assert profile_payload == {
                "profiles": [
                    {
                        "id": "local",
                        "label": "Installed Local",
                        "provider": "openai_compatible",
                        "models": ["installed-local"],
                        "default_model": "installed-local",
                        "configured": False,
                    }
                ]
            }
            assert "from-provider-env" not in profiles.text
            missing = client.get("/workspace/api/missing")
            assert missing.status_code == 404
            assert missing.headers["content-type"] == "application/json"
            assert missing.json() == {"detail": "Not Found"}
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        stdout.close()
        stderr.close()

    process_env = {**server_env, profile_key: "from-process-env"}
    port = _free_tcp_port()
    stdout = (tmp_path / "installed-serve-process-env.stdout.log").open("w", encoding="utf-8")
    stderr = (tmp_path / "installed-serve-process-env.stderr.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(spark_server_bin),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=tmp_path,
        env=process_env,
        text=True,
        stdout=stdout,
        stderr=stderr,
    )
    base_url = f"http://127.0.0.1:{port}"
    try:
        _wait_for_server(base_url, process)
        with httpx.Client(base_url=base_url, timeout=10.0) as client:
            profiles = client.get("/attractor/api/llm-profiles")
            assert profiles.status_code == 200
            assert profiles.json()["profiles"][0]["configured"] is True
            assert "from-process-env" not in profiles.text
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        stdout.close()
        stderr.close()

    fake_systemctl_dir = tmp_path / "fake-bin"
    fake_systemctl_log = tmp_path / "systemctl-calls.jsonl"
    xdg_config_home = tmp_path / "xdg-config"
    ui_dir = tmp_path / "service-ui"
    ui_dir.mkdir()
    (ui_dir / "index.html").write_text("<!doctype html><html></html>\n", encoding="utf-8")
    _write_fake_systemctl(fake_systemctl_dir / "systemctl")
    provider_env.write_text(
        "OPENAI_API_KEY=compat-sentinel\nOPENROUTER_API_KEY=compat-sentinel\n",
        encoding="utf-8",
    )
    service_env = {
        **server_env,
        "PATH": os.pathsep.join([str(fake_systemctl_dir), default_env.get("PATH", "")]),
        "XDG_CONFIG_HOME": str(xdg_config_home),
        "SPARK_COMPAT_SYSTEMCTL_LOG": str(fake_systemctl_log),
    }
    service_unit_path = xdg_config_home / "systemd/user/spark.service"
    service_install = _run_process(
        [
            str(spark_server_bin),
            "service",
            "install",
            "--host",
            "127.0.0.1",
            "--port",
            "8123",
            "--data-dir",
            str(data_dir),
            "--flows-dir",
            str(flows_dir),
            "--ui-dir",
            str(ui_dir),
        ],
        cwd=tmp_path,
        env=service_env,
        timeout=60,
    )
    assert service_install["returncode"] == 0, service_install["stderr"]
    assert "Listening on http://127.0.0.1:8123" in service_install["stdout"]
    unit_text = service_unit_path.read_text(encoding="utf-8")
    assert "Type=simple\n" in unit_text
    assert "Restart=on-failure\n" in unit_text
    assert "RestartSec=2\n" in unit_text
    assert "WantedBy=default.target\n" in unit_text
    assert f"EnvironmentFile=-{provider_env}" in unit_text
    assert f"Environment=SPARK_HOME={data_dir}\n" in unit_text
    assert f"Environment=SPARK_FLOWS_DIR={flows_dir}\n" in unit_text
    assert f"Environment=SPARK_UI_DIR={ui_dir}\n" in unit_text
    assert (
        f"ExecStart={rust_binary_paths['spark-server']} serve --host 127.0.0.1 "
        f"--port 8123 --data-dir {data_dir} --flows-dir {flows_dir} --ui-dir {ui_dir}\n"
    ) in unit_text
    assert provider_env.read_text(encoding="utf-8").startswith("OPENAI_API_KEY=compat-sentinel")

    service_status = _run_process(
        [str(spark_server_bin), "service", "status"],
        cwd=tmp_path,
        env=service_env,
        timeout=30,
    )
    assert service_status["returncode"] == 0, service_status["stderr"]
    assert "Active: active (running)" in service_status["stdout"]

    service_remove = _run_process(
        [str(spark_server_bin), "service", "remove"],
        cwd=tmp_path,
        env=service_env,
        timeout=30,
    )
    assert service_remove["returncode"] == 0, service_remove["stderr"]
    assert not service_unit_path.exists()
    assert [
        invocation["user_args"]
        for invocation in _read_fake_systemctl_invocations(fake_systemctl_log)
    ] == [
        ["daemon-reload"],
        ["enable", "spark.service"],
        ["restart", "spark.service"],
        ["--no-pager", "--full", "status", "spark.service"],
        ["disable", "--now", "spark.service"],
        ["daemon-reload"],
        ["reset-failed", "spark.service"],
    ]


def test_m6_compose_package_config_preserves_packaged_runtime_contract(
    tmp_path: Path,
    rewrite_worktree_path: Path,
) -> None:
    if not _docker_compose_available():
        pytest.skip("Docker Compose plugin is unavailable")

    spark_home = tmp_path / "spark-docker-home"
    projects_dir = tmp_path / "projects"
    env = {
        **os.environ,
        "HOME": str(tmp_path / "account-home"),
        "SPARK_DOCKER_HOME": str(spark_home),
        "SPARK_PROJECTS_HOST_DIR": str(projects_dir),
        "SPARK_DOCKER_HOST_UID": "1234",
        "SPARK_DOCKER_HOST_GID": "5678",
    }
    for key in M6_DOCKER_PROVIDER_ENV_KEYS:
        env[key] = f"{key.lower()}-sentinel"

    config = _docker_compose_config(rewrite_worktree_path, env)
    service = _compose_service(config)
    service_env = _compose_service_environment(service)
    volumes = _compose_service_volumes(service)
    ports = _compose_service_ports(service)

    assert service.get("image") == "spark:package"
    assert service_env["SPARK_HOME"] == "/spark"
    assert service_env["SPARK_PROJECT_ROOTS"] == "/projects"
    assert service_env["SPARK_DOCKER_HOME"] == str(spark_home)
    assert service_env["SPARK_PROJECTS_HOST_DIR"] == str(projects_dir)
    assert service_env["SPARK_DOCKER_HOST_UID"] == "1234"
    assert service_env["SPARK_DOCKER_HOST_GID"] == "5678"
    assert service_env["ATTRACTOR_CODEX_RUNTIME_ROOT"] == "/spark/runtime/codex"
    assert service_env["HOME"] == "/spark/runtime/codex"
    assert service_env["CODEX_HOME"] == "/spark/runtime/codex/.codex"
    assert service_env["XDG_CONFIG_HOME"] == "/spark/runtime/codex/.config"
    assert service_env["XDG_DATA_HOME"] == "/spark/runtime/codex/.local/share"
    for key in M6_DOCKER_PROVIDER_ENV_KEYS:
        assert service_env[key] == f"{key.lower()}-sentinel"
    assert any(volume["source"] == str(spark_home) and volume["target"] == "/spark" for volume in volumes)
    assert any(volume["source"] == str(projects_dir) and volume["target"] == "/projects" for volume in volumes)
    assert any(volume["source"] == "/var/run/docker.sock" and volume["target"] == "/var/run/docker.sock" for volume in volumes)
    assert any(port["target"] == 8000 and str(port["published"]) == "8000" for port in ports)
    assert service.get("user") == "1234:5678"


def test_m6_runtime_docker_image_starts_packaged_app(
    tmp_path: Path,
    rewrite_worktree_path: Path,
) -> None:
    if not _docker_daemon_available():
        pytest.skip("Docker daemon is unavailable")

    image_tag = f"spark-m6-i10-runtime-{os.getpid()}-{tmp_path.name}".lower().replace("_", "-")
    build = _run_process(
        ["docker", "build", "-t", image_tag, "."],
        cwd=rewrite_worktree_path,
        timeout=1200,
    )
    assert build["returncode"] == 0, build["stderr"][-4000:] or build["stdout"][-4000:]

    spark_home = tmp_path / "spark-home"
    projects_dir = tmp_path / "projects"
    spark_home.joinpath("config").mkdir(parents=True)
    projects_dir.mkdir()
    profile_key = "SPARK_COMPAT_PROFILE_KEY"
    spark_home.joinpath("config/llm-profiles.toml").write_text(
        """
[profiles.local]
label = "Docker Local"
provider = "openai_compatible"
base_url = "http://127.0.0.1:65535/v1"
api_key_env = "SPARK_COMPAT_PROFILE_KEY"
models = ["docker-local"]
default_model = "docker-local"
""".strip(),
        encoding="utf-8",
    )

    host_port = _free_tcp_port()
    container_name = f"{image_tag}-smoke"
    run = _run_process(
        [
            "docker",
            "run",
            "--rm",
            "-d",
            "--name",
            container_name,
            "--user",
            f"{os.getuid()}:{os.getgid()}",
            "-e",
            "SPARK_HOME=/spark",
            "-e",
            "SPARK_PROJECT_ROOTS=/projects",
            "-e",
            "ATTRACTOR_CODEX_RUNTIME_ROOT=/spark/runtime/codex",
            "-e",
            f"{profile_key}=from-process-env",
            "-v",
            f"{spark_home}:/spark",
            "-v",
            f"{projects_dir}:/projects",
            "-p",
            f"127.0.0.1:{host_port}:8000",
            image_tag,
        ],
        cwd=rewrite_worktree_path,
        timeout=60,
    )
    assert run["returncode"] == 0, run["stderr"]
    base_url = f"http://127.0.0.1:{host_port}"
    try:
        _wait_for_container_server(base_url, container_name)
        with httpx.Client(base_url=base_url, timeout=10.0) as client:
            index = client.get("/")
            assert index.status_code == 200
            assert index.headers["content-type"].startswith("text/html")
            assert "<!doctype html" in index.text.lower()
            favicon = client.get("/favicon.ico")
            assert favicon.status_code == 200
            assert favicon.headers["content-type"] == "image/png"
            assert favicon.content
            settings = client.get("/workspace/api/settings")
            assert settings.status_code == 200
            assert settings.json()["execution_placement"]["profiles"][0]["id"] == "native"
            attractor_status = client.get("/attractor/status")
            assert attractor_status.status_code == 200
            assert attractor_status.json()["status"] == "idle"
            profiles = client.get("/attractor/api/llm-profiles")
            assert profiles.status_code == 200
            assert profiles.json() == {
                "profiles": [
                    {
                        "id": "local",
                        "label": "Docker Local",
                        "provider": "openai_compatible",
                        "models": ["docker-local"],
                        "default_model": "docker-local",
                        "configured": True,
                    }
                ]
            }
            assert "from-process-env" not in profiles.text
            assert "127.0.0.1:65535" not in profiles.text
            missing = client.get("/workspace/api/missing")
            assert missing.status_code == 404
            assert missing.headers["content-type"] == "application/json"
            assert missing.json() == {"detail": "Not Found"}

        assert spark_home.joinpath("config/flow-catalog.toml").is_file()
        assert spark_home.joinpath("flows/examples/simple-linear.dot").is_file()

        for command in ("spark", "spark-server"):
            help_result = _run_process(
                ["docker", "exec", container_name, command, "--help"],
                cwd=rewrite_worktree_path,
                timeout=30,
            )
            assert help_result["returncode"] == 0, help_result["stderr"]
            assert command in help_result["stdout"]

        no_source_tree = _run_process(
            [
                "docker",
                "exec",
                container_name,
                "sh",
                "-lc",
                "for path in /app/src /src /app/frontend /app/target /app/node_modules; do test ! -e \"$path\"; done",
            ],
            cwd=rewrite_worktree_path,
            timeout=30,
        )
        assert no_source_tree["returncode"] == 0, no_source_tree["stderr"]
    finally:
        _run_process(["docker", "rm", "-f", container_name], cwd=rewrite_worktree_path, timeout=30)
        _run_process(["docker", "image", "rm", "-f", image_tag], cwd=rewrite_worktree_path, timeout=60)


@pytest.fixture(scope="session")
def packaging_build_smoke(
    tmp_path_factory: pytest.TempPathFactory,
    rewrite_worktree_path: Path,
) -> BuildSmokeObservation:
    output_dir = tmp_path_factory.mktemp("compat-deliverable-output")
    missing = [
        command
        for command in ("npm", "uv")
        if shutil.which(command) is None
    ]
    if missing:
        return BuildSmokeObservation(
            manifest=_packaging_manifest(
                fixture_id="packaging/build-deliverable-smoke",
                scenario="build_deliverable_smoke",
                provenance_interfaces=["scripts/build_deliverable.py"],
                skipped_prerequisite={
                    "reason": "required command unavailable",
                    "commands": missing,
                },
            ),
            wheel_path=None,
        )

    env = {
        **os.environ,
        "SPARK_DELIVERABLE_OUT": str(output_dir),
    }
    process = _run_process(
        [
            "uv",
            "run",
            "python",
            "scripts/build_deliverable.py",
        ],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=240,
    )
    wheel_paths = sorted(output_dir.glob("spark-*.whl"))
    sdist_paths = sorted(output_dir.glob("spark-*.tar.gz"))
    wheel_path = wheel_paths[0] if process["returncode"] == 0 and wheel_paths else None
    artifacts = {
        "output_dir": str(output_dir),
        "produced": [
            _artifact_summary(path)
            for path in [*wheel_paths, *sdist_paths]
        ],
        "wheel": _wheel_artifact_observation(wheel_path),
    }
    manifest = _packaging_manifest(
        fixture_id="packaging/build-deliverable-smoke",
        scenario="build_deliverable_smoke",
        provenance_interfaces=[
            "uv run python scripts/build_deliverable.py",
            "npm --prefix frontend run build",
            "uv build",
            "wheel content inspection",
        ],
        command={
            "argv": process["argv"],
            "cwd": process["cwd"],
            "environment": {"SPARK_DELIVERABLE_OUT": str(output_dir)},
        },
        process=_build_process_summary(process),
        artifacts=artifacts,
    )
    manifest = harness.normalize_path_tokens(
        manifest,
        {
            "__DELIVERABLE_OUT__": output_dir,
            "__WORKTREE__": rewrite_worktree_path,
        },
    )
    return BuildSmokeObservation(manifest=manifest, wheel_path=wheel_path)


@contextmanager
def _running_source_ui_server(
    *,
    tmp_path: Path,
    rewrite_worktree_path: Path,
    source_ui: CompatSourceUi,
) -> Iterator[dict[str, Any]]:
    root = tmp_path / "source-ui-server"
    spark_home = root / "spark-home"
    flows_dir = root / "flows"
    runtime_root = root / "codex-runtime"
    codex_home = root / "codex-home"
    logs_dir = root / "logs"
    for path in (spark_home, flows_dir, runtime_root, codex_home, logs_dir):
        path.mkdir(parents=True, exist_ok=True)
    env = {
        **os.environ,
        "SPARK_HOME": str(spark_home),
        "SPARK_FLOWS_DIR": str(flows_dir),
        "SPARK_UI_DIR": str(source_ui.root),
        "ATTRACTOR_CODEX_RUNTIME_ROOT": str(runtime_root),
        "CODEX_HOME": str(codex_home),
    }
    init = subprocess.run(
        [
            "uv",
            "run",
            "spark-server",
            "init",
            "--data-dir",
            str(spark_home),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=rewrite_worktree_path,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=60,
    )
    assert init.returncode == 0, init.stderr
    port = _free_tcp_port()
    stdout = (logs_dir / "stdout.log").open("w", encoding="utf-8")
    stderr = (logs_dir / "stderr.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            "uv",
            "run",
            "spark-server",
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(spark_home),
            "--flows-dir",
            str(flows_dir),
            "--ui-dir",
            str(source_ui.root),
        ],
        cwd=rewrite_worktree_path,
        env=env,
        text=True,
        stdout=stdout,
        stderr=stderr,
    )
    base_url = f"http://127.0.0.1:{port}"
    try:
        _wait_for_server(base_url, process)
        yield {
            "base_url": base_url,
            "spark_home": spark_home,
            "flows_dir": flows_dir,
            "ui_dir": source_ui.root,
            "process": process,
        }
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        stdout.close()
        stderr.close()


def _installed_command_manifest(*, tmp_path: Path, wheel_path: Path | None) -> dict[str, Any]:
    if wheel_path is None:
        return _packaging_manifest(
            fixture_id="packaging/installed-command-init-smoke",
            scenario="installed_command_init_smoke",
            provenance_interfaces=["python venv install", "spark --help", "spark-server init"],
            skipped_prerequisite={
                "reason": "build deliverable smoke did not produce a wheel",
            },
        )
    venv_dir = tmp_path / "installed-venv"
    builder = venv.EnvBuilder(with_pip=True, system_site_packages=True)
    builder.create(venv_dir)
    python_bin = _venv_bin(venv_dir, "python")
    spark_bin = _venv_bin(venv_dir, "spark")
    spark_server_bin = _venv_bin(venv_dir, "spark-server")
    install = _run_process(
        [
            str(python_bin),
            "-m",
            "pip",
            "install",
            str(wheel_path),
        ],
        cwd=tmp_path,
        timeout=120,
    )
    spark_help = _run_process([str(spark_bin), "--help"], cwd=tmp_path, timeout=30)
    server_help = _run_process([str(spark_server_bin), "--help"], cwd=tmp_path, timeout=30)
    spark_home = tmp_path / "installed-spark-home"
    flows_dir = tmp_path / "installed-flows"
    init = _run_process(
        [
            str(spark_server_bin),
            "init",
            "--data-dir",
            str(spark_home),
            "--flows-dir",
            str(flows_dir),
        ],
        cwd=tmp_path,
        env={
            **os.environ,
            "SPARK_HOME": str(spark_home),
            "SPARK_FLOWS_DIR": str(flows_dir),
        },
        timeout=60,
    )
    resources = _installed_resource_probe(python_bin)
    manifest = _packaging_manifest(
        fixture_id="packaging/installed-command-init-smoke",
        scenario="installed_command_init_smoke",
        provenance_interfaces=[
            "python -m pip install built wheel",
            "spark --help",
            "spark-server --help",
            "spark-server init --data-dir --flows-dir",
            "importlib.resources from installed wheel",
        ],
        artifacts={"wheel": _artifact_summary(wheel_path)},
        observations={
            "install": _pip_install_summary(install),
            "spark_help": _process_excerpt(spark_help),
            "spark_server_help": _process_excerpt(server_help),
            "spark_server_init": _process_excerpt(init),
            "seeded_flows": sorted(
                path.relative_to(flows_dir).as_posix()
                for path in flows_dir.rglob("*.dot")
            )
            if flows_dir.exists()
            else [],
            "installed_resources": resources,
        },
    )
    return harness.normalize_path_tokens(
        manifest,
        {
            "__INSTALL_ROOT__": tmp_path,
            "__VENV__": venv_dir,
            "__SPARK_HOME__": spark_home,
            "__SPARK_FLOWS_DIR__": flows_dir,
            "__WHEEL__": wheel_path,
        },
    )


def _packaging_manifest(
    *,
    fixture_id: str,
    scenario: str,
    provenance_interfaces: list[str],
    command: Mapping[str, Any] | None = None,
    process: Mapping[str, Any] | None = None,
    server: Mapping[str, Any] | None = None,
    requests: list[Mapping[str, Any]] | None = None,
    artifacts: Mapping[str, Any] | None = None,
    resources: Mapping[str, Any] | None = None,
    observations: Mapping[str, Any] | None = None,
    skipped_prerequisite: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    manifest: dict[str, Any] = {
        "schema_version": "compat-packaging-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I05,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-packaging-smoke",
            "interfaces": provenance_interfaces,
        },
        "scenario": scenario,
    }
    if command is not None:
        manifest["command"] = dict(command)
    if process is not None:
        manifest["process"] = dict(process)
    if server is not None:
        manifest["server"] = dict(server)
    if requests is not None:
        manifest["requests"] = [dict(request) for request in requests]
    if artifacts is not None:
        manifest["artifacts"] = dict(artifacts)
    if resources is not None:
        manifest["resources"] = dict(resources)
    if observations is not None:
        manifest["observations"] = dict(observations)
    if skipped_prerequisite is not None:
        manifest["skipped_prerequisite"] = dict(skipped_prerequisite)
    return manifest


def _assert_packaging_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=ITEM_REQUIREMENTS,
        decision_ids=ITEM_DECISIONS,
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_packaging_manifest_matches_golden(manifest, expected)


def _run_process(
    argv: list[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    timeout: float,
) -> dict[str, Any]:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        env=dict(env) if env is not None else None,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    return {
        "argv": argv,
        "cwd": str(cwd),
        "returncode": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }


def _docker_compose_available() -> bool:
    if shutil.which("docker") is None:
        return False
    result = subprocess.run(
        ["docker", "compose", "version"],
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    return result.returncode == 0


def _docker_daemon_available() -> bool:
    if shutil.which("docker") is None:
        return False
    result = subprocess.run(
        ["docker", "info"],
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    return result.returncode == 0


def _docker_compose_config(
    rewrite_worktree_path: Path,
    env: Mapping[str, str],
) -> dict[str, Any]:
    result = _run_process(
        ["docker", "compose", "-f", "compose.package.yaml", "config", "--format", "json"],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=60,
    )
    if result["returncode"] == 0:
        return json.loads(str(result["stdout"]))
    fallback = _run_process(
        ["docker", "compose", "-f", "compose.package.yaml", "config"],
        cwd=rewrite_worktree_path,
        env=env,
        timeout=60,
    )
    assert fallback["returncode"] == 0, fallback["stderr"]
    return _compose_text_model(str(fallback["stdout"]), env)


def _compose_service(config: Mapping[str, Any]) -> dict[str, Any]:
    services = config.get("services")
    assert isinstance(services, Mapping)
    service = services.get("spark")
    assert isinstance(service, Mapping)
    return dict(service)


def _compose_service_environment(service: Mapping[str, Any]) -> dict[str, str]:
    environment = service.get("environment", {})
    if isinstance(environment, Mapping):
        return {str(key): "" if value is None else str(value) for key, value in environment.items()}
    if isinstance(environment, list):
        values: dict[str, str] = {}
        for entry in environment:
            key, _, value = str(entry).partition("=")
            values[key] = value
        return values
    raise AssertionError(f"unexpected Compose environment shape: {environment!r}")


def _compose_service_volumes(service: Mapping[str, Any]) -> list[dict[str, str]]:
    normalized: list[dict[str, str]] = []
    for volume in service.get("volumes", []):
        if isinstance(volume, Mapping):
            normalized.append(
                {
                    "source": str(volume.get("source", "")),
                    "target": str(volume.get("target", "")),
                }
            )
            continue
        source, _, target_and_mode = str(volume).partition(":")
        target, _, _mode = target_and_mode.partition(":")
        normalized.append({"source": source, "target": target})
    return normalized


def _compose_service_ports(service: Mapping[str, Any]) -> list[dict[str, Any]]:
    normalized: list[dict[str, Any]] = []
    for port in service.get("ports", []):
        if isinstance(port, Mapping):
            normalized.append(
                {
                    "target": int(port.get("target", port.get("target_port", 0))),
                    "published": str(port.get("published", port.get("published_port", ""))),
                }
            )
            continue
        published, _, target = str(port).rpartition(":")
        normalized.append({"target": int(target.split("/", 1)[0]), "published": published})
    return normalized


def _compose_text_model(output: str, env: Mapping[str, str]) -> dict[str, Any]:
    service_env = {
        "SPARK_HOME": "/spark",
        "SPARK_PROJECT_ROOTS": "/projects",
        "SPARK_DOCKER_HOME": str(env["SPARK_DOCKER_HOME"]),
        "SPARK_DOCKER_HOST_UID": str(env["SPARK_DOCKER_HOST_UID"]),
        "SPARK_DOCKER_HOST_GID": str(env["SPARK_DOCKER_HOST_GID"]),
        "SPARK_PROJECTS_HOST_DIR": str(env["SPARK_PROJECTS_HOST_DIR"]),
        "ATTRACTOR_CODEX_RUNTIME_ROOT": "/spark/runtime/codex",
        "HOME": "/spark/runtime/codex",
        "CODEX_HOME": "/spark/runtime/codex/.codex",
        "XDG_CONFIG_HOME": "/spark/runtime/codex/.config",
        "XDG_DATA_HOME": "/spark/runtime/codex/.local/share",
    }
    service_env.update({key: str(env[key]) for key in M6_DOCKER_PROVIDER_ENV_KEYS})
    for key, value in service_env.items():
        assert f"{key}: {value}" in output or f"- {key}={value}" in output
    return {
        "services": {
            "spark": {
                "image": "spark:package" if "image: spark:package" in output else "",
                "environment": service_env,
                "volumes": [
                    {"source": str(env["SPARK_DOCKER_HOME"]), "target": "/spark"},
                    {"source": str(env["SPARK_PROJECTS_HOST_DIR"]), "target": "/projects"},
                    {"source": "/var/run/docker.sock", "target": "/var/run/docker.sock"},
                ],
                "ports": [{"target": 8000, "published": "8000"}],
                "user": f"{env['SPARK_DOCKER_HOST_UID']}:{env['SPARK_DOCKER_HOST_GID']}",
            }
        }
    }


def _without_spark_path_env(env: Mapping[str, str]) -> dict[str, str]:
    return {
        key: value
        for key, value in env.items()
        if key not in {"SPARK_HOME", "SPARK_FLOWS_DIR", "SPARK_UI_DIR"}
    }


def _process_excerpt(process: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "argv": process["argv"],
        "cwd": process["cwd"],
        "returncode": process["returncode"],
        "stdout_lines": str(process.get("stdout", "")).splitlines()[:12],
        "stderr_lines": str(process.get("stderr", "")).splitlines()[:12],
    }


def _build_process_summary(process: Mapping[str, Any]) -> dict[str, Any]:
    stdout = str(process.get("stdout", ""))
    stderr = str(process.get("stderr", ""))
    return {
        "argv": process["argv"],
        "cwd": process["cwd"],
        "returncode": process["returncode"],
        "stdout_observation": {
            "ran_frontend_build": "+ npm --prefix frontend run build" in stdout,
            "ran_uv_build": "+ uv build" in stdout,
            "reported_deliverable_ready": "deliverable ready:" in stdout,
            "reported_vite_build": "vite " in stdout and "building client environment" in stdout,
        },
        "stderr_observation": {
            "reported_chunk_size_warning": "Some chunks are larger than 500 kB" in stderr,
            "reported_sdist_success": "Successfully built dist/spark-0.1.0.tar.gz" in stderr,
            "reported_wheel_success": re.search(
                r"Successfully built dist/spark-0\.1\.0-[^.]+-[^.]+-[^.]+\.whl",
                stderr,
            )
            is not None,
        },
    }


def _pip_install_summary(process: Mapping[str, Any]) -> dict[str, Any]:
    stdout = str(process.get("stdout", ""))
    stderr = str(process.get("stderr", ""))
    return {
        "argv": process["argv"],
        "cwd": process["cwd"],
        "returncode": process["returncode"],
        "stdout_observation": {
            "processed_wheel": "Processing __WHEEL__" in stdout or "Processing " in stdout,
            "installed_spark": "Successfully installed" in stdout and "spark-0.1.0" in stdout,
            "resolved_fastapi": "fastapi" in stdout,
            "resolved_httpx": "httpx" in stdout,
            "resolved_uvicorn": "uvicorn" in stdout,
        },
        "stderr_observation": {
            "line_count": len(stderr.splitlines()),
        },
    }


def _artifact_summary(path: Path) -> dict[str, Any]:
    return {
        "artifact_name": path.name,
        "kind": "wheel" if path.suffix == ".whl" else "sdist",
        "size": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def _wheel_artifact_observation(wheel_path: Path | None) -> dict[str, Any]:
    if wheel_path is None:
        return {"exists": False}
    required = [
        "spark/ui_dist/index.html",
        "spark/guides/dot-authoring.md",
        "spark/guides/spark-operations.md",
        "spark/flows/examples/simple-linear.dot",
        "spark/flows/software-development/spec-implementation/implement-spec.dot",
        "unified_llm/data/models.json",
    ]
    forbidden = [
        "spark/guides/attractor-spec.md",
        "spark/guides/spark-flow-extensions.md",
    ]
    with zipfile.ZipFile(wheel_path) as wheel:
        names = set(wheel.namelist())
        ui_entries = sorted(
            name
            for name in names
            if name.startswith("spark/ui_dist/")
            and not name.endswith("/")
        )
    return {
        "exists": True,
        "filename": wheel_path.name,
        "required_entries": {entry: entry in names for entry in required},
        "forbidden_entries_absent": {entry: entry not in names for entry in forbidden},
        "ui_entry_count": len(ui_entries),
        "selected_ui_entries": _selected_ui_entries(ui_entries),
    }


def _wheel_resource_presence(wheel_path: Path | None) -> dict[str, Any]:
    if wheel_path is None:
        return {"checked": False, "reason": "no wheel produced"}
    observation = _wheel_artifact_observation(wheel_path)
    observation["checked"] = True
    return observation


def _artifact_forbidden_entries(entries: set[str]) -> list[str]:
    allowed_test_entries = {
        "tests/compat/providers/test_unified_llm_adapter_fixtures.py",
        "tests/compat/packaging/test_packaging_smoke_fixtures.py",
    }
    allowed_test_dirs = {
        "tests",
        "tests/compat",
        "tests/compat/providers",
        "tests/compat/packaging",
    }
    forbidden_dir_names = {
        ".spark",
        ".venv",
        "__tests__",
        "__pycache__",
        ".pytest_cache",
        ".ruff_cache",
        "dist",
        "node_modules",
        "target",
        "tests",
    }
    forbidden_prefixes = {
        ("frontend", "artifacts"),
        ("frontend", "dist"),
        ("frontend", "e2e"),
        ("frontend", "node_modules"),
        ("frontend", "playwright-report"),
        ("frontend", "src", "test"),
        ("frontend", "test-results"),
    }
    forbidden: list[str] = []
    for entry in entries:
        relative_entry = _artifact_relative_name(entry)
        if relative_entry in allowed_test_entries or relative_entry in allowed_test_dirs:
            continue
        parts = Path(entry).parts
        if any(part in forbidden_dir_names for part in parts):
            forbidden.append(entry)
            continue
        if entry.endswith((".pyc", ".pyo")):
            forbidden.append(entry)
            continue
        for index in range(len(parts)):
            suffix = parts[index:]
            if any(suffix[: len(prefix)] == prefix for prefix in forbidden_prefixes):
                forbidden.append(entry)
                break
    return sorted(forbidden)


def _artifact_relative_name(entry: str) -> str:
    parts = Path(entry).parts
    if len(parts) > 1 and parts[0].startswith("spark-"):
        return Path(*parts[1:]).as_posix()
    return Path(entry).as_posix()


def _sdist_relative_names(entries: set[str]) -> set[str]:
    relative_names: set[str] = set()
    for entry in entries:
        parts = Path(entry).parts
        if len(parts) > 1 and parts[0].startswith("spark-"):
            relative_names.add(Path(*parts[1:]).as_posix())
        else:
            relative_names.add(Path(entry).as_posix())
    return relative_names


def _wheel_metadata(wheel: zipfile.ZipFile) -> dict[str, Any]:
    metadata_entries = [
        name for name in wheel.namelist() if name.endswith(".dist-info/WHEEL")
    ]
    assert len(metadata_entries) == 1
    tags: list[str] = []
    root_is_purelib = ""
    for line in wheel.read(metadata_entries[0]).decode("utf-8").splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.lower() == "root-is-purelib":
            root_is_purelib = value.strip().lower()
        elif key.lower() == "tag":
            tags.append(value.strip())
    return {
        "root_is_purelib": root_is_purelib,
        "tags": tags,
    }


def _native_wheel_tag() -> str:
    return f"py3-none-{sysconfig.get_platform().replace('-', '_').replace('.', '_')}"


def _is_native_binary_payload(payload: bytes) -> bool:
    return payload.startswith(
        (
            b"\x7fELF",
            b"MZ",
            b"\xca\xfe\xba\xbe",
            b"\xfe\xed\xfa\xce",
            b"\xfe\xed\xfa\xcf",
            b"\xce\xfa\xed\xfe",
            b"\xcf\xfa\xed\xfe",
        )
    )


def _installed_rust_binary_paths(python_bin: Path) -> dict[str, Path]:
    script = """
import importlib.resources
import json
spark_root = importlib.resources.files("spark")
print(json.dumps({
    "spark": str(spark_root.joinpath("bin/spark")),
    "spark-server": str(spark_root.joinpath("bin/spark-server")),
}, sort_keys=True))
""".strip()
    result = subprocess.run(
        [str(python_bin), "-c", script],
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    assert result.returncode == 0, result.stderr
    return {
        str(command_name): Path(path)
        for command_name, path in json.loads(result.stdout).items()
    }


def _selected_ui_entries(entries: list[str]) -> list[str]:
    selected = [entry for entry in entries if entry == "spark/ui_dist/index.html"]
    selected.extend(entry for entry in entries if re.search(r"assets/.+\.(js|css|png)$", entry))
    return selected[:8]


def _source_package_resources() -> dict[str, Any]:
    spark_root = importlib.resources.files("spark")
    unified_llm_root = importlib.resources.files("unified_llm")
    checks = {
        "flows/examples/simple-linear.dot": spark_root.joinpath(
            "flows/examples/simple-linear.dot"
        ).is_file(),
        "flows/software-development/implement-change-request.dot": spark_root.joinpath(
            "flows/software-development/implement-change-request.dot"
        ).is_file(),
        "guides/dot-authoring.md": spark_root.joinpath("guides/dot-authoring.md").is_file(),
        "guides/spark-operations.md": spark_root.joinpath(
            "guides/spark-operations.md"
        ).is_file(),
        "ui_dist/index.html": spark_root.joinpath("ui_dist/index.html").is_file(),
        "unified_llm/data/models.json": unified_llm_root.joinpath(
            "data/models.json"
        ).is_file(),
    }
    return {
        "resource_presence": checks,
        "installed_asset_closure_gate": "M6",
    }


def _installed_resource_probe(python_bin: Path) -> dict[str, Any]:
    script = """
import importlib.resources
import json
spark_root = importlib.resources.files("spark")
llm_root = importlib.resources.files("unified_llm")
print(json.dumps({
    "spark/ui_dist/index.html": spark_root.joinpath("ui_dist/index.html").is_file(),
    "spark/guides/dot-authoring.md": spark_root.joinpath("guides/dot-authoring.md").is_file(),
    "spark/guides/spark-operations.md": spark_root.joinpath("guides/spark-operations.md").is_file(),
    "spark/flows/examples/simple-linear.dot": spark_root.joinpath("flows/examples/simple-linear.dot").is_file(),
    "unified_llm/data/models.json": llm_root.joinpath("data/models.json").is_file(),
}, sort_keys=True))
""".strip()
    result = subprocess.run(
        [str(python_bin), "-c", script],
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    if result.returncode != 0:
        return {
            "returncode": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }
    return json.loads(result.stdout)


def _write_fake_systemctl(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(
            [
                f"#!{sys.executable}",
                "import json",
                "import os",
                "import sys",
                "from pathlib import Path",
                "",
                "log_path = Path(os.environ['SPARK_COMPAT_SYSTEMCTL_LOG'])",
                "log_path.parent.mkdir(parents=True, exist_ok=True)",
                "entry = {'argv': sys.argv, 'args': sys.argv[1:], 'cwd': os.getcwd()}",
                "with log_path.open('a', encoding='utf-8') as stream:",
                "    stream.write(json.dumps(entry, sort_keys=True) + '\\n')",
                "",
                "args = sys.argv[1:]",
                "user_args = args[1:] if args[:1] == ['--user'] else args",
                "if user_args[:3] == ['--no-pager', '--full', 'status']:",
                "    print('spark.service - Spark compatibility fake status')",
                "    print('   Active: active (running)')",
                "elif user_args[:1] == ['daemon-reload']:",
                "    print('compat daemon-reload')",
                "elif user_args[:1] == ['enable']:",
                "    print('compat enable ' + ' '.join(user_args[1:]))",
                "elif user_args[:1] == ['restart']:",
                "    print('compat restart ' + ' '.join(user_args[1:]))",
                "elif user_args[:1] == ['disable']:",
                "    print('compat disable ' + ' '.join(user_args[1:]))",
                "elif user_args[:1] == ['reset-failed']:",
                "    print('compat reset-failed ' + ' '.join(user_args[1:]))",
                "else:",
                "    print('compat systemctl ' + ' '.join(user_args))",
                "sys.exit(0)",
                "",
            ]
        ),
        encoding="utf-8",
    )
    path.chmod(0o755)


def _read_fake_systemctl_invocations(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    invocations: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        entry = json.loads(line)
        args = [str(value) for value in entry.get("args", [])]
        user_args = args[1:] if args[:1] == ["--user"] else args
        invocations.append(
            {
                "argv": [str(value) for value in entry.get("argv", [])],
                "user_args": user_args,
                "cwd": str(entry.get("cwd", "")),
            }
        )
    return invocations


def _service_unit_observation(unit_text: str) -> dict[str, Any]:
    lines = unit_text.splitlines()
    return {
        "sections": [line for line in lines if line.startswith("[") and line.endswith("]")],
        "environment_keys": sorted(
            line.split("=", 1)[1].split("=", 1)[0].strip('"')
            for line in lines
            if line.startswith("Environment=")
        ),
        "has_environment_file": any(line.startswith("EnvironmentFile=-") for line in lines),
        "exec_start": next((line for line in lines if line.startswith("ExecStart=")), ""),
        "wanted_by": next((line for line in lines if line.startswith("WantedBy=")), ""),
    }


def _venv_bin(venv_dir: Path, name: str) -> Path:
    bin_dir = "Scripts" if sys.platform == "win32" else "bin"
    suffix = ".exe" if sys.platform == "win32" and name != "python" else ""
    return venv_dir / bin_dir / f"{name}{suffix}"


def _free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _wait_for_server(base_url: str, process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 30
    last_error = ""
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"spark-server exited early with {process.returncode}")
        try:
            response = httpx.get(f"{base_url}/workspace/api/projects", timeout=1.0)
            if response.status_code == 200:
                return
            last_error = response.text
        except httpx.HTTPError as exc:
            last_error = str(exc)
        time.sleep(0.2)
    raise AssertionError(f"spark-server did not become ready: {last_error}")


def _wait_for_container_server(base_url: str, container_name: str) -> None:
    deadline = time.monotonic() + 60
    last_error = ""
    while time.monotonic() < deadline:
        inspect = subprocess.run(
            [
                "docker",
                "inspect",
                "-f",
                "{{.State.Running}} {{.State.ExitCode}}",
                container_name,
            ],
            text=True,
            capture_output=True,
            check=False,
            timeout=10,
        )
        if inspect.returncode == 0 and inspect.stdout.strip().startswith("false"):
            logs = subprocess.run(
                ["docker", "logs", container_name],
                text=True,
                capture_output=True,
                check=False,
                timeout=10,
            )
            raise AssertionError(
                "spark runtime container exited before serving HTTP:\n"
                f"{logs.stdout[-4000:]}\n{logs.stderr[-4000:]}"
            )
        try:
            response = httpx.get(f"{base_url}/workspace/api/projects", timeout=1.0)
            if response.status_code == 200:
                return
            last_error = response.text
        except httpx.HTTPError as exc:
            last_error = str(exc)
        time.sleep(0.5)
    logs = subprocess.run(
        ["docker", "logs", container_name],
        text=True,
        capture_output=True,
        check=False,
        timeout=10,
    )
    raise AssertionError(
        "spark runtime container did not become ready: "
        f"{last_error}\n{logs.stdout[-4000:]}\n{logs.stderr[-4000:]}"
    )
