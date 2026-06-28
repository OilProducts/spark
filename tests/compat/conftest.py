from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
from typing import Any, Callable, Mapping

import httpx
import pytest

from tests.compat import harness
from spark_common.project_identity import build_project_id


ITEM_REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
ITEM_DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")
ITEM_ID_M0_I04 = "M0-I04-DSL-RUNTIME-FIXTURES"
ITEM_ID_M0_I05 = "M0-I05-FRONTEND-PACKAGING-FIXTURES"


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--compat-update-goldens",
        action="store_true",
        default=False,
        help="rewrite reviewed compatibility golden fixtures from current observations",
    )


@dataclass(frozen=True)
class CompatSandbox:
    root: Path
    spark_home: Path
    flows_dir: Path
    runtime_root: Path
    codex_home: Path
    output_path: Path
    project_dir: Path
    valid_flow_path: Path
    validation_error_flow_path: Path
    invalid_flow_path: Path
    messy_flow_path: Path
    trigger_payload_path: Path
    dsl_sources_dir: Path
    runtime_sources_dir: Path
    env: dict[str, str]

    def capture_output(self, fixture_id: str) -> Path:
        safe_name = fixture_id.replace("/", "-").replace("\\", "-")
        return self.root / "captures" / f"{safe_name}.json"

    def capture_command(
        self,
        *,
        rewrite_worktree_path: Path,
        fixture_id: str,
        item_id: str,
        requirements: tuple[str, ...],
        decisions: tuple[str, ...],
        command: list[str],
        output: Path | None = None,
        snapshot_roots: dict[str, str | Path] | None = None,
        state_files: dict[str, str | Path] | None = None,
        normalize_paths: dict[str, str | Path] | None = None,
        command_env: dict[str, str] | None = None,
        unset_command_env: tuple[str, ...] = (),
        no_default_path_env: bool = False,
        timeout: float = 60.0,
    ) -> list[str]:
        argv = [
            sys.executable,
            str(rewrite_worktree_path / "scripts" / "compat_capture.py"),
            "--fixture-id",
            fixture_id,
            "--item-id",
            item_id,
            "--spark-home",
            str(self.spark_home),
            "--flows-dir",
            str(self.flows_dir),
            "--runtime-root",
            str(self.runtime_root),
            "--codex-home",
            str(self.codex_home),
            "--validation-root",
            str(self.root / "validation"),
            "--timeout",
            str(timeout),
            "--output",
            str(output or self.capture_output(fixture_id)),
        ]
        for requirement in requirements:
            argv.extend(["--requirement", requirement])
        for decision in decisions:
            argv.extend(["--decision", decision])
        for name, path in (normalize_paths or {}).items():
            argv.extend(["--normalize-path", f"{name}={path}"])
        for name, path in (snapshot_roots or {}).items():
            argv.extend(["--snapshot-root", f"{name}={path}"])
        for name, path in (state_files or {}).items():
            argv.extend(["--state-file", f"{name}={path}"])
        for key, value in (command_env or {}).items():
            argv.extend(["--command-env", f"{key}={value}"])
        for key in unset_command_env:
            argv.extend(["--unset-command-env", key])
        if no_default_path_env:
            argv.append("--no-default-path-env")
        argv.append("--")
        argv.extend(command)
        return argv


@dataclass
class CompatServer:
    root: Path
    spark_home: Path
    flows_dir: Path
    runtime_root: Path
    codex_home: Path
    project_dir: Path
    base_url: str
    process: subprocess.Popen[str]
    env: dict[str, str]
    conversation_id: str
    conversation_handle: str
    trigger_payload_path: Path
    oracle: str = "python-spark-server"
    server_argv_override: list[str] | None = None

    def capture_output(self, fixture_id: str) -> Path:
        safe_name = fixture_id.replace("/", "-").replace("\\", "-")
        return self.root / "captures" / f"{safe_name}.json"

    def request_manifest(
        self,
        *,
        fixture_id: str,
        item_id: str,
        method: str,
        path: str,
        requirements: tuple[str, ...] = ITEM_REQUIREMENTS,
        decisions: tuple[str, ...] = ITEM_DECISIONS,
        params: Mapping[str, Any] | None = None,
        headers: Mapping[str, str] | None = None,
        json_body: Any = None,
        text_body: str | None = None,
        timeout: float = 10.0,
    ) -> dict[str, Any]:
        request_kwargs: dict[str, Any] = {
            "params": dict(params or {}),
            "headers": dict(headers or {}),
        }
        request_body: dict[str, Any] | None = None
        if json_body is not None:
            request_kwargs["json"] = json_body
            request_body = {"kind": "json", "json": json_body}
        elif text_body is not None:
            request_kwargs["content"] = text_body
            request_body = {"kind": "text", "text": text_body}
        with httpx.Client(base_url=self.base_url, timeout=timeout) as client:
            response = client.request(method, path, **request_kwargs)
        manifest = self._route_manifest(
            schema_version="compat-http-v1",
            fixture_id=fixture_id,
            item_id=item_id,
            requirements=requirements,
            decisions=decisions,
            request={
                "method": method.upper(),
                "path": path,
                "query": dict(params or {}),
                "headers": dict(headers or {}),
                "body": request_body,
            },
            response={
                "status_code": response.status_code,
                "headers": harness.selected_http_headers(response.headers),
                "body": harness.http_body_observation(
                    content=response.content,
                    headers=response.headers,
                ),
            },
        )
        return self._normalize_route_manifest(manifest)

    def sse_manifest(
        self,
        *,
        fixture_id: str,
        item_id: str,
        path: str = "/workspace/api/live/events",
        requirements: tuple[str, ...] = ITEM_REQUIREMENTS,
        decisions: tuple[str, ...] = ITEM_DECISIONS,
        params: Mapping[str, Any] | None = None,
        frame_limit: int = 1,
        after_connect: Callable[[], None] | None = None,
        after_frame: Mapping[int, Callable[[], None]] | None = None,
        read_timeout: float = 5.0,
    ) -> dict[str, Any]:
        timeout = httpx.Timeout(5.0, connect=5.0, read=read_timeout, write=5.0, pool=5.0)
        raw_frames: list[str] = []
        response_payload: dict[str, Any]
        with httpx.Client(base_url=self.base_url, timeout=timeout) as client:
            with client.stream("GET", path, params=dict(params or {})) as response:
                response_payload = {
                    "status_code": response.status_code,
                    "headers": harness.selected_http_headers(response.headers),
                }
                content_type = response.headers.get("content-type", "")
                if response.status_code != 200 or "text/event-stream" not in content_type:
                    content = response.read()
                    response_payload["body"] = harness.http_body_observation(
                        content=content,
                        headers=response.headers,
                    )
                else:
                    if after_connect is not None:
                        after_connect()
                    current_frame: list[str] = []
                    for line in response.iter_lines():
                        if line == "":
                            raw_frames.append("\n".join(current_frame))
                            current_frame = []
                            callback = (after_frame or {}).get(len(raw_frames))
                            if callback is not None:
                                callback()
                            if len(raw_frames) >= frame_limit:
                                break
                            continue
                        current_frame.append(line)
        manifest = self._route_manifest(
            schema_version="compat-sse-v1",
            fixture_id=fixture_id,
            item_id=item_id,
            requirements=requirements,
            decisions=decisions,
            request={
                "method": "GET",
                "path": path,
                "query": dict(params or {}),
                "headers": {},
                "body": None,
            },
            response=response_payload,
            frames=harness.parse_sse_frames(raw_frames),
        )
        return self._normalize_route_manifest(manifest)

    def seed_http_flow(self, name: str, content: str) -> str:
        relative = Path(name)
        flow_path = self.flows_dir / relative
        flow_path.parent.mkdir(parents=True, exist_ok=True)
        flow_path.write_text(content, encoding="utf-8")
        return relative.as_posix()

    def capture_command(
        self,
        *,
        rewrite_worktree_path: Path,
        fixture_id: str,
        item_id: str,
        requirements: tuple[str, ...],
        decisions: tuple[str, ...],
        command: list[str],
        output: Path | None = None,
        snapshot_roots: dict[str, str | Path] | None = None,
        state_files: dict[str, str | Path] | None = None,
        normalize_paths: dict[str, str | Path] | None = None,
        timeout: float = 60.0,
    ) -> list[str]:
        sandbox = CompatSandbox(
            root=self.root,
            spark_home=self.spark_home,
            flows_dir=self.flows_dir,
            runtime_root=self.runtime_root,
            codex_home=self.codex_home,
            output_path=self.capture_output(fixture_id),
            project_dir=self.project_dir,
            valid_flow_path=self.flows_dir / "software-development" / "implement-change-request.dot",
            validation_error_flow_path=self.flows_dir / "examples" / "simple-linear.dot",
            invalid_flow_path=self.flows_dir / "missing.dot",
            messy_flow_path=self.flows_dir / "examples" / "simple-linear.dot",
            trigger_payload_path=self.trigger_payload_path,
            dsl_sources_dir=self.root / "dsl-sources",
            runtime_sources_dir=self.root / "runtime-sources",
            env=self.env,
        )
        return sandbox.capture_command(
            rewrite_worktree_path=rewrite_worktree_path,
            fixture_id=fixture_id,
            item_id=item_id,
            requirements=requirements,
            decisions=decisions,
            command=command,
            output=output or self.capture_output(fixture_id),
            snapshot_roots=snapshot_roots,
            state_files=state_files,
            normalize_paths={
                "PROJECT_DIR": self.project_dir,
                "API_BASE_URL": self.base_url,
                **(normalize_paths or {}),
            },
            command_env={"SPARK_API_BASE_URL": self.base_url},
            timeout=timeout,
        )

    def _route_manifest(
        self,
        *,
        schema_version: str,
        fixture_id: str,
        item_id: str,
        requirements: tuple[str, ...],
        decisions: tuple[str, ...],
        request: dict[str, Any],
        response: dict[str, Any],
        frames: list[dict[str, Any]] | None = None,
    ) -> dict[str, Any]:
        manifest: dict[str, Any] = {
            "schema_version": schema_version,
            "fixture_id": fixture_id,
            "item_id": item_id,
            "requirements": list(requirements),
            "decisions": list(decisions),
            "provenance": {
                "oracle": self.oracle,
                "server_argv": self._server_argv(),
                "environment": _selected_server_environment(self.env),
            },
            "server": {
                "base_url": self.base_url,
                "spark_home": str(self.spark_home),
                "flows_dir": str(self.flows_dir),
                "runtime_root": str(self.runtime_root),
                "codex_home": str(self.codex_home),
            },
            "request": request,
            "response": response,
        }
        if frames is not None:
            manifest["frames"] = frames
        return manifest

    def _normalize_route_manifest(self, manifest: Mapping[str, Any]) -> dict[str, Any]:
        return harness.normalize_path_tokens(
            manifest,
            {
                "__SPARK_HOME__": self.spark_home,
                "__SPARK_FLOWS_DIR__": self.flows_dir,
                "__ATTRACTOR_CODEX_RUNTIME_ROOT__": self.runtime_root,
                "__CODEX_HOME__": self.codex_home,
                "__PROJECT_DIR__": self.project_dir,
            },
        )

    def _server_argv(self) -> list[str]:
        if self.server_argv_override is not None:
            return list(self.server_argv_override)
        return [
            "uv",
            "run",
            "spark-server",
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            self.base_url.rsplit(":", maxsplit=1)[-1],
            "--data-dir",
            str(self.spark_home),
            "--flows-dir",
            str(self.flows_dir),
        ]


@dataclass(frozen=True)
class CompatSourceUi:
    root: Path
    index_path: Path
    asset_path: Path
    favicon_path: Path


@pytest.fixture(scope="session")
def rewrite_worktree_path() -> Path:
    return Path(__file__).resolve().parents[2]


@pytest.fixture(scope="session")
def compat_fixture_root(rewrite_worktree_path: Path) -> Path:
    root = rewrite_worktree_path / "tests" / "compat" / "fixtures"
    assert root.is_dir(), f"missing committed compatibility fixture root: {root}"
    return root


@pytest.fixture
def compat_validation_root(tmp_path: Path) -> Path:
    root = tmp_path / "compat-validation"
    root.mkdir(parents=True, exist_ok=True)
    return root


@pytest.fixture
def compat_update_goldens(pytestconfig: pytest.Config) -> bool:
    return bool(pytestconfig.getoption("--compat-update-goldens")) or os.environ.get(
        "SPARK_COMPAT_UPDATE_GOLDENS"
    ) == "1"


@pytest.fixture
def compat_source_ui_dir(tmp_path: Path) -> CompatSourceUi:
    ui_root = tmp_path / "source-ui"
    assets_dir = ui_root / "assets"
    assets_dir.mkdir(parents=True, exist_ok=True)
    index_path = ui_root / "index.html"
    asset_path = assets_dir / "compat-app.js"
    favicon_path = assets_dir / "spark-app-icon.png"
    index_path.write_text(
        "\n".join(
            [
                "<!doctype html>",
                "<html>",
                "  <head><title>Spark Compat UI</title></head>",
                "  <body><main id=\"root\">Spark compatibility UI</main></body>",
                "</html>",
                "",
            ]
        ),
        encoding="utf-8",
    )
    asset_path.write_text(
        "window.__sparkCompatAsset = { status: 'ok' };\n",
        encoding="utf-8",
    )
    favicon_path.write_bytes(b"spark-compat-favicon\n")
    return CompatSourceUi(
        root=ui_root,
        index_path=index_path,
        asset_path=asset_path,
        favicon_path=favicon_path,
    )


@pytest.fixture
def compat_sandbox(
    compat_validation_root: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> CompatSandbox:
    root = compat_validation_root / "sandbox"
    spark_home = root / "spark-home"
    flows_dir = root / "flows"
    runtime_root = root / "codex-runtime"
    codex_home = root / "codex-home"
    output_path = root / "captures" / "manifest.json"
    project_dir = root / "project"
    valid_flow_path = project_dir / "valid-flow.dot"
    validation_error_flow_path = project_dir / "validation-error-flow.dot"
    invalid_flow_path = project_dir / "invalid-flow.dot"
    messy_flow_path = project_dir / "messy-flow.dot"
    trigger_payload_path = project_dir / "trigger-payload.json"
    dsl_sources_dir = root / "dsl-sources"
    runtime_sources_dir = root / "runtime-sources"
    for path in (
        spark_home,
        flows_dir,
        runtime_root,
        codex_home,
        output_path.parent,
        project_dir,
        dsl_sources_dir,
        runtime_sources_dir,
    ):
        path.mkdir(parents=True, exist_ok=True)
    _write_compat_flow_files(
        valid_flow_path=valid_flow_path,
        validation_error_flow_path=validation_error_flow_path,
        invalid_flow_path=invalid_flow_path,
        messy_flow_path=messy_flow_path,
    )
    trigger_payload_path.write_text(
        json.dumps(
            {
                "name": "Compat webhook",
                "enabled": True,
                "source_type": "webhook",
                "action": {
                    "flow_name": "software-development/implement-change-request.dot",
                    "project_path": str(project_dir),
                    "static_context": {"origin": "compat"},
                },
                "source": {},
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    env = os.environ.copy()
    env.update(
        {
            "SPARK_HOME": str(spark_home),
            "SPARK_FLOWS_DIR": str(flows_dir),
            "ATTRACTOR_CODEX_RUNTIME_ROOT": str(runtime_root),
            "CODEX_HOME": str(codex_home),
            "SPARK_COMPAT_FAKE_PROVIDER_KEY": "compat-fixture-placeholder",
        }
    )
    for key, value in env.items():
        if key.startswith(("SPARK_", "ATTRACTOR_", "CODEX_")):
            monkeypatch.setenv(key, value)

    return CompatSandbox(
        root=root,
        spark_home=spark_home,
        flows_dir=flows_dir,
        runtime_root=runtime_root,
        codex_home=codex_home,
        output_path=output_path,
        project_dir=project_dir,
        valid_flow_path=valid_flow_path,
        validation_error_flow_path=validation_error_flow_path,
        invalid_flow_path=invalid_flow_path,
        messy_flow_path=messy_flow_path,
        trigger_payload_path=trigger_payload_path,
        dsl_sources_dir=dsl_sources_dir,
        runtime_sources_dir=runtime_sources_dir,
        env=env,
    )


@pytest.fixture
def compat_server(tmp_path: Path, rewrite_worktree_path: Path) -> CompatServer:
    root = tmp_path / "compat-server"
    spark_home = root / "spark-home"
    flows_dir = root / "flows"
    runtime_root = root / "codex-runtime"
    codex_home = root / "codex-home"
    project_dir = root / "project"
    logs_dir = root / "server-logs"
    captures_dir = root / "captures"
    for path in (spark_home, flows_dir, runtime_root, codex_home, project_dir, logs_dir, captures_dir):
        path.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env.update(
        {
            "SPARK_HOME": str(spark_home),
            "SPARK_FLOWS_DIR": str(flows_dir),
            "ATTRACTOR_CODEX_RUNTIME_ROOT": str(runtime_root),
            "CODEX_HOME": str(codex_home),
        }
    )
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
        with httpx.Client(base_url=base_url, timeout=5.0) as client:
            policy_response = client.put(
                "/workspace/api/flows/examples/simple-linear.dot/launch-policy",
                json={"launch_policy": "agent_requestable"},
            )
            assert policy_response.status_code == 200, policy_response.text
            conversation_id = "conversation-compat"
            settings_response = client.put(
                f"/workspace/api/conversations/{conversation_id}/settings",
                json={"project_path": str(project_dir), "chat_mode": "chat"},
            )
            assert settings_response.status_code == 200, settings_response.text
            conversation_handle = str(settings_response.json()["conversation_handle"])
        _seed_assistant_turn(spark_home, project_dir, conversation_id)

        trigger_payload_path = project_dir / "trigger-payload.json"
        trigger_payload_path.write_text(
            json.dumps(
                {
                    "name": "Compat webhook",
                    "enabled": True,
                    "source_type": "webhook",
                    "action": {
                        "flow_name": "software-development/implement-change-request.dot",
                        "project_path": str(project_dir),
                        "static_context": {"origin": "compat"},
                    },
                    "source": {},
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        yield CompatServer(
            root=root,
            spark_home=spark_home,
            flows_dir=flows_dir,
            runtime_root=runtime_root,
            codex_home=codex_home,
            project_dir=project_dir,
            base_url=base_url,
            process=process,
            env=env,
            conversation_id=conversation_id,
            conversation_handle=conversation_handle,
            trigger_payload_path=trigger_payload_path,
        )
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        stdout.close()
        stderr.close()


@pytest.fixture(scope="session")
def rust_spark_server_binary(rewrite_worktree_path: Path) -> Path:
    subprocess.run(
        ["cargo", "build", "-p", "spark-server", "--bin", "spark-server"],
        cwd=rewrite_worktree_path,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return rewrite_worktree_path / "target" / "debug" / "spark-server"


@pytest.fixture
def rust_compat_server(
    tmp_path: Path,
    rewrite_worktree_path: Path,
    rust_spark_server_binary: Path,
) -> CompatServer:
    root = tmp_path / "rust-compat-server"
    spark_home = root / "spark-home"
    flows_dir = root / "flows"
    runtime_root = root / "codex-runtime"
    codex_home = root / "codex-home"
    project_dir = root / "project"
    logs_dir = root / "server-logs"
    captures_dir = root / "captures"
    for path in (spark_home, flows_dir, runtime_root, codex_home, project_dir, logs_dir, captures_dir):
        path.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env.update(
        {
            "SPARK_HOME": str(spark_home),
            "SPARK_FLOWS_DIR": str(flows_dir),
            "ATTRACTOR_CODEX_RUNTIME_ROOT": str(runtime_root),
            "CODEX_HOME": str(codex_home),
        }
    )
    init = subprocess.run(
        [
            str(rust_spark_server_binary),
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
    )
    assert init.returncode == 0, init.stderr

    port = _free_tcp_port()
    stdout = (logs_dir / "stdout.log").open("w", encoding="utf-8")
    stderr = (logs_dir / "stderr.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(rust_spark_server_binary),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(spark_home),
            "--flows-dir",
            str(flows_dir),
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
        trigger_payload_path = project_dir / "trigger-payload.json"
        trigger_payload_path.write_text(
            json.dumps(
                {
                    "name": "Compat webhook",
                    "enabled": True,
                    "source_type": "webhook",
                    "action": {
                        "flow_name": "software-development/implement-change-request.dot",
                        "project_path": str(project_dir),
                        "static_context": {"origin": "compat"},
                    },
                    "source": {},
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        yield CompatServer(
            root=root,
            spark_home=spark_home,
            flows_dir=flows_dir,
            runtime_root=runtime_root,
            codex_home=codex_home,
            project_dir=project_dir,
            base_url=base_url,
            process=process,
            env=env,
            conversation_id="",
            conversation_handle="",
            trigger_payload_path=trigger_payload_path,
            oracle="rust-spark-server",
            server_argv_override=[
                "target/debug/spark-server",
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                str(port),
                "--data-dir",
                str(spark_home),
                "--flows-dir",
                str(flows_dir),
            ],
        )
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        stdout.close()
        stderr.close()


def _write_compat_flow_files(
    *,
    valid_flow_path: Path,
    validation_error_flow_path: Path,
    invalid_flow_path: Path,
    messy_flow_path: Path,
) -> None:
    valid_flow_path.write_text(
        """digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box, prompt="Do work"];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
""",
        encoding="utf-8",
    )
    validation_error_flow_path.write_text(
        """digraph Workflow {
  task [shape=box, prompt="No start or done"];
}
""",
        encoding="utf-8",
    )
    invalid_flow_path.write_text("digraph Workflow { start -> }\n", encoding="utf-8")
    messy_flow_path.write_text(
        """
digraph Workflow {
  done [shape=Msquare];
  task [shape=box, prompt="Do work"];
  start [shape=Mdiamond];
  task -> done;
  start -> task;
}
""",
        encoding="utf-8",
    )


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


def _selected_server_environment(env: Mapping[str, str]) -> dict[str, str]:
    selected: dict[str, str] = {}
    for key, value in sorted(env.items()):
        if key in {
            "SPARK_HOME",
            "SPARK_FLOWS_DIR",
            "ATTRACTOR_CODEX_RUNTIME_ROOT",
            "CODEX_HOME",
        }:
            selected[key] = value
    return selected


def _seed_assistant_turn(spark_home: Path, project_dir: Path, conversation_id: str) -> None:
    project_id = build_project_id(str(project_dir))
    state_path = (
        spark_home
        / "workspace"
        / "projects"
        / project_id
        / "conversations"
        / conversation_id
        / "state.json"
    )
    state = json.loads(state_path.read_text(encoding="utf-8"))
    state.setdefault("turns", []).append(
        {
            "id": "turn-assistant-compat",
            "role": "assistant",
            "content": "Ready for a compatibility run request.",
            "timestamp": "2026-01-01T00:00:00Z",
            "status": "complete",
            "kind": "message",
        }
    )
    state["revision"] = int(state.get("revision") or 0) + 1
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
