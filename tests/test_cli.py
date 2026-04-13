from __future__ import annotations

from argparse import Namespace
import json
from pathlib import Path

import pytest

import spark.authoring_assets as authoring_assets
import spark.cli as spark_cli
import spark.starter_assets as starter_assets
from spark.ui import resolve_default_ui_dir
import spark.server_cli as spark_server_cli


TEST_AGENT_FLOW = "test-dispatch.dot"


@pytest.fixture(autouse=True)
def _disable_source_checkout_guard_by_default(monkeypatch) -> None:
    monkeypatch.setattr(spark_cli, "_running_from_source_checkout", lambda _project_root: False)


def test_run_serve_uses_import_string_when_reload_enabled(monkeypatch, tmp_path: Path) -> None:
    calls: list[dict[str, object]] = []

    def fake_run(app, **kwargs):
        calls.append({"app": app, **kwargs})

    monkeypatch.setattr("uvicorn.run", fake_run)

    args = Namespace(
        host="127.0.0.1",
        port=8000,
        reload=True,
        data_dir=tmp_path / "data",
        flows_dir=tmp_path / "flows",
        ui_dir=tmp_path / "ui",
        command="serve",
    )
    args.ui_dir.mkdir(parents=True, exist_ok=True)
    (args.ui_dir / "index.html").write_text("<html></html>", encoding="utf-8")

    result = spark_server_cli._run_serve(args)

    assert result == 0
    assert len(calls) == 1
    assert isinstance(calls[0]["app"], str)
    assert calls[0]["host"] == "127.0.0.1"
    assert calls[0]["port"] == 8000
    assert calls[0]["reload"] is True


def test_run_serve_preserves_runtime_path_env_for_reload(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr("uvicorn.run", lambda *args, **kwargs: None)

    data_dir = tmp_path / "data"
    flows_dir = tmp_path / "flows"
    ui_dir = tmp_path / "ui"
    ui_dir.mkdir(parents=True, exist_ok=True)
    (ui_dir / "index.html").write_text("<html></html>", encoding="utf-8")

    args = Namespace(
        host="127.0.0.1",
        port=8000,
        reload=True,
        data_dir=data_dir,
        flows_dir=flows_dir,
        ui_dir=ui_dir,
        command="serve",
    )

    spark_server_cli._run_serve(args)

    assert spark_server_cli.os.environ["SPARK_HOME"] == str(data_dir.resolve(strict=False))
    assert spark_server_cli.os.environ["SPARK_FLOWS_DIR"] == str(flows_dir.resolve(strict=False))
    assert spark_server_cli.os.environ["SPARK_UI_DIR"] == str(ui_dir.resolve(strict=False))


def test_run_serve_requires_explicit_dev_home_from_source_checkout(
    monkeypatch,
    capsys,
) -> None:
    monkeypatch.setattr(spark_server_cli, "_running_from_source_checkout", lambda _project_root: True)
    monkeypatch.delenv("SPARK_HOME", raising=False)

    result = spark_server_cli.main(["serve"])

    assert result == 1
    stderr = capsys.readouterr().err
    assert "Refusing to use default runtime home ~/.spark from a source checkout" in stderr
    assert "SPARK_HOME=~/.spark-dev uv run spark-server serve --reload --port 8010" in stderr


def test_run_init_seeds_missing_starter_flows_without_overwriting_existing(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flows_dir = tmp_path / "flows"
    existing_flow = flows_dir / "examples" / "parallel-review.dot"
    existing_flow.parent.mkdir(parents=True, exist_ok=True)
    existing_flow.write_text("custom-parallel\n", encoding="utf-8")

    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("examples/parallel-review.dot", "canonical-parallel\n"),
            starter_assets.StarterFlowAsset("examples/simple-linear.dot", "simple-linear\n"),
        ),
    )

    result = spark_server_cli.main(
        [
            "init",
            "--data-dir",
            str(tmp_path / "data"),
            "--flows-dir",
            str(flows_dir),
        ]
    )

    assert result == 0
    assert existing_flow.read_text(encoding="utf-8") == "custom-parallel\n"
    assert (flows_dir / "examples" / "simple-linear.dot").read_text(encoding="utf-8") == "simple-linear\n"
    output = capsys.readouterr().out
    assert "created=1 updated=0 skipped=1" in output


def test_run_init_force_overwrites_existing_starter_flows(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flows_dir = tmp_path / "flows"
    existing_flow = flows_dir / "examples" / "parallel-review.dot"
    existing_flow.parent.mkdir(parents=True, exist_ok=True)
    existing_flow.write_text("custom-parallel\n", encoding="utf-8")

    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("examples/parallel-review.dot", "canonical-parallel\n"),
            starter_assets.StarterFlowAsset("examples/simple-linear.dot", "simple-linear\n"),
        ),
    )

    result = spark_server_cli.main(
        [
            "init",
            "--data-dir",
            str(tmp_path / "data"),
            "--flows-dir",
            str(flows_dir),
            "--force",
        ]
    )

    assert result == 0
    assert existing_flow.read_text(encoding="utf-8") == "canonical-parallel\n"
    assert (flows_dir / "examples" / "simple-linear.dot").read_text(encoding="utf-8") == "simple-linear\n"
    output = capsys.readouterr().out
    assert "created=1 updated=1 skipped=0" in output


def test_run_init_creates_nested_starter_flow_directories(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(parents=True, exist_ok=True)

    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("examples/supervision/implementation-worker.dot", "worker\n"),
            starter_assets.StarterFlowAsset("examples/supervision/supervised-implementation.dot", "parent\n"),
        ),
    )

    result = spark_server_cli.main(
        [
            "init",
            "--data-dir",
            str(tmp_path / "data"),
            "--flows-dir",
            str(flows_dir),
        ]
    )

    assert result == 0
    assert (flows_dir / "examples" / "supervision" / "implementation-worker.dot").read_text(encoding="utf-8") == "worker\n"
    assert (flows_dir / "examples" / "supervision" / "supervised-implementation.dot").read_text(encoding="utf-8") == "parent\n"
    output = capsys.readouterr().out
    assert "created=2 updated=0 skipped=0" in output


def test_run_init_requires_explicit_dev_home_from_source_checkout(
    monkeypatch,
    capsys,
) -> None:
    monkeypatch.setattr(spark_server_cli, "_running_from_source_checkout", lambda _project_root: True)
    monkeypatch.delenv("SPARK_HOME", raising=False)

    result = spark_server_cli.main(["init"])

    assert result == 1
    stderr = capsys.readouterr().err
    assert "Refusing to use default runtime home ~/.spark from a source checkout" in stderr
    assert "SPARK_HOME=~/.spark-dev uv run spark-server init" in stderr


def test_run_init_allows_explicit_home_env_from_source_checkout(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    monkeypatch.setattr(spark_server_cli, "_running_from_source_checkout", lambda _project_root: True)
    monkeypatch.delenv("SPARK_FLOWS_DIR", raising=False)
    monkeypatch.delenv("SPARK_UI_DIR", raising=False)
    monkeypatch.setenv("SPARK_HOME", str(tmp_path / "dev-home"))
    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("examples/simple-linear.dot", "simple-linear\n"),
        ),
    )

    result = spark_server_cli.main(["init"])

    assert result == 0
    output = capsys.readouterr().out
    assert f"Initialized Spark at {(tmp_path / 'dev-home').resolve(strict=False)}" in output
    assert f"Seeded flows: {(tmp_path / 'dev-home' / 'flows').resolve(strict=False)}" in output


def test_packaged_starter_flows_exist_in_single_source_tree() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    packaged_starter_dir = repo_root / "src" / "spark" / "flows"

    packaged_payload = {
        path.relative_to(packaged_starter_dir).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(packaged_starter_dir.rglob("*.dot"))
    }

    assert "examples/simple-linear.dot" in packaged_payload
    assert "spec-implementation/implement-spec.dot" in packaged_payload


def test_packaged_guides_are_available_and_repo_only_specs_are_not_packaged() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    packaged_authoring_guide = authoring_assets.dot_authoring_guide_path()
    packaged_operations_guide = authoring_assets.spark_operations_guide_path()

    assert packaged_authoring_guide.read_text(encoding="utf-8").startswith("# Spark DOT Authoring Guide")
    assert packaged_operations_guide.read_text(encoding="utf-8").startswith("# Spark Operations Guide")
    assert not (repo_root / "src" / "spark" / "guides" / "attractor-spec.md").exists()
    assert not (repo_root / "src" / "spark" / "guides" / "spark-flow-extensions.md").exists()


def test_packaged_starter_flows_are_loadable_without_repo_checkout(tmp_path: Path) -> None:
    assets = starter_assets.load_starter_flow_assets(project_root=tmp_path)

    payload = {asset.name: asset.content for asset in assets}

    assert "examples/simple-linear.dot" in payload
    assert "spec-implementation/implement-spec.dot" in payload
    assert payload["spec-implementation/implement-spec.dot"]


def test_resolve_default_ui_dir_prefers_repo_frontend_dist(tmp_path: Path) -> None:
    source_dist = tmp_path / "frontend" / "dist"
    source_dist.mkdir(parents=True, exist_ok=True)
    (source_dist / "index.html").write_text("<html></html>", encoding="utf-8")

    ui_dir = resolve_default_ui_dir(tmp_path)

    assert ui_dir == source_dist.resolve(strict=False)


def test_agent_run_request_posts_payload_and_prints_response(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> dict[str, object]:
            return {
                "ok": True,
                "conversation_id": "conversation-123",
                "conversation_handle": "amber-otter",
                "flow_run_request_id": "flow-run-request-123",
                "segment_id": "segment-artifact-flow-run-request-123",
            }

    calls: list[tuple[str, dict[str, object]]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            calls.append((f"{method} {url}", json or {}))
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(
        [
            "convo",
            "run-request",
            "--conversation",
            "amber-otter",
            "--flow",
            TEST_AGENT_FLOW,
            "--summary",
            "Run implementation for the approved scope",
            "--goal",
            "Implement the approved work items.",
            "--launch-context-json",
            '{"context.request.summary":"Implement the approved work items."}',
            "--model",
            "gpt-5",
        ]
    )

    assert result == 0
    assert calls == [
        (
            "POST http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/flow-run-requests",
            {
                "flow_name": TEST_AGENT_FLOW,
                "summary": "Run implementation for the approved scope",
                "goal": "Implement the approved work items.",
                "launch_context": {"context.request.summary": "Implement the approved work items."},
                "model": "gpt-5",
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["flow_run_request_id"] == "flow-run-request-123"


def test_agent_run_launch_requires_project_without_conversation(capsys) -> None:
    result = spark_cli.main(
        [
            "run",
            "launch",
            "--flow",
            TEST_AGENT_FLOW,
            "--summary",
            "Launch directly",
        ]
    )

    assert result == spark_cli.EXIT_GENERAL_FAILURE
    assert "--project when --conversation is omitted" in capsys.readouterr().err


def test_agent_run_launch_posts_payload_and_prints_response(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> dict[str, object]:
            return {
                "ok": True,
                "status": "started",
                "run_id": "run-123",
                "flow_name": TEST_AGENT_FLOW,
                "project_path": "/tmp/project",
                "conversation_id": "conversation-123",
                "conversation_handle": "amber-otter",
                "flow_launch_id": "flow-launch-123",
                "segment_id": "segment-artifact-flow-launch-123",
            }

    calls: list[tuple[str, dict[str, object]]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            calls.append((f"{method} {url}", json or {}))
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(
        [
            "run",
            "launch",
            "--flow",
            TEST_AGENT_FLOW,
            "--summary",
            "Launch directly",
            "--conversation",
            "amber-otter",
            "--project",
            "/tmp/project",
            "--goal",
            "Implement the approved work items.",
        ]
    )

    assert result == 0
    assert calls == [
        (
            "POST http://127.0.0.1:8000/workspace/api/runs/launch",
            {
                "flow_name": TEST_AGENT_FLOW,
                "summary": "Launch directly",
                "goal": "Implement the approved work items.",
                "conversation_handle": "amber-otter",
                "project_path": "/tmp/project",
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["flow_launch_id"] == "flow-launch-123"


def test_agent_flow_list_defaults_to_json(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return [
                {
                    "name": TEST_AGENT_FLOW,
                    "title": "Implement spec",
                    "description": "Execute approved work items.",
                    "launch_policy": "agent_requestable",
                    "effective_launch_policy": "agent_requestable",
                }
            ]

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            assert method == "GET"
            assert json is None
            assert url == "http://127.0.0.1:8000/workspace/api/flows?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "list", "--base-url", "http://127.0.0.1:8000"])

    assert result == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload[0]["name"] == TEST_AGENT_FLOW


def test_agent_describe_flow_text_mode_formats_human_readable_output(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return {
                "name": TEST_AGENT_FLOW,
                "title": "Implement Spec",
                "description": "Execute approved work items.",
                "effective_launch_policy": "agent_requestable",
                "graph_label": "Implement Spec",
                "graph_goal": "Implement approved changes",
                "node_count": 5,
                "edge_count": 4,
                "features": {
                    "has_human_gate": True,
                    "has_manager_loop": False,
                },
            }

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            assert url == f"http://127.0.0.1:8000/workspace/api/flows/{TEST_AGENT_FLOW}?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "describe", "--flow", TEST_AGENT_FLOW, "--text"])

    assert result == 0
    output = capsys.readouterr().out
    assert f"Name: {TEST_AGENT_FLOW}" in output
    assert "Launch Policy: agent_requestable" in output


def test_agent_validate_flow_text_mode_formats_diagnostics(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return {
                "name": TEST_AGENT_FLOW,
                "path": f"/flows/{TEST_AGENT_FLOW}",
                "status": "invalid",
                "diagnostics": [
                    {
                        "severity": "error",
                        "rule_id": "missing-edge",
                        "message": "Missing edge.",
                        "line": 7,
                    }
                ],
                "errors": ["Missing edge."],
            }

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            assert url == f"http://127.0.0.1:8000/workspace/api/flows/{TEST_AGENT_FLOW}/validate"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "validate", "--flow", TEST_AGENT_FLOW, "--text"])

    assert result == 0
    output = capsys.readouterr().out
    assert "Status: invalid" in output
    assert "- ERROR missing-edge line 7: Missing edge." in output


def test_agent_validate_flow_file_text_mode_uses_local_preview(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flow_path = tmp_path / "broken.dot"
    flow_path.write_text("digraph broken { start -> }\n", encoding="utf-8")

    class UnexpectedClient:
        def __init__(self, *args, **kwargs) -> None:
            raise AssertionError("local --file validation should not open an HTTP client")

    monkeypatch.setattr(spark_cli.httpx, "Client", UnexpectedClient)

    result = spark_cli.main(["flow", "validate", "--file", str(flow_path), "--text"])

    assert result == 0
    output = capsys.readouterr().out
    assert f"Name: {flow_path.name}" in output
    assert f"Path: {flow_path.resolve(strict=False)}" in output
    assert "Status: parse_error" in output


def test_agent_validate_flow_file_reports_missing_path(capsys) -> None:
    result = spark_cli.main(["flow", "validate", "--file", "/tmp/does-not-exist.dot"])

    assert result == spark_cli.EXIT_GENERAL_FAILURE
    assert "Flow file not found: /tmp/does-not-exist.dot" in capsys.readouterr().err


def test_agent_cli_requires_explicit_base_url_from_source_checkout(
    monkeypatch,
    capsys,
) -> None:
    monkeypatch.setattr(spark_cli, "_running_from_source_checkout", lambda _project_root: True)
    monkeypatch.delenv("SPARK_API_BASE_URL", raising=False)

    result = spark_cli.main(["flow", "list"])

    assert result == spark_cli.EXIT_GENERAL_FAILURE
    stderr = capsys.readouterr().err
    assert "Refusing to use default API target http://127.0.0.1:8000 from a source checkout" in stderr
    assert "SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow list" in stderr


def test_agent_cli_allows_explicit_base_url_env_from_source_checkout(monkeypatch, capsys) -> None:
    monkeypatch.setattr(spark_cli, "_running_from_source_checkout", lambda _project_root: True)
    monkeypatch.setenv("SPARK_API_BASE_URL", "http://127.0.0.1:8010")

    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            assert method == "GET"
            assert json is None
            assert url == "http://127.0.0.1:8010/workspace/api/flows?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "list"])

    assert result == 0
    assert json.loads(capsys.readouterr().out) == []


def test_agent_get_flow_defaults_to_json_wrapper(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False
        text = "digraph G {\n  a -> b;\n}\n"

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            assert url == f"http://127.0.0.1:8000/workspace/api/flows/{TEST_AGENT_FLOW}/raw?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "get", "--flow", TEST_AGENT_FLOW])

    assert result == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["name"] == TEST_AGENT_FLOW
    assert "a -> b" in payload["content"]


def test_agent_flow_discovery_returns_not_found_exit_code_on_404(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 404
        is_error = True
        text = "Not found"

        def json(self) -> object:
            return {"detail": "Unknown flow: missing.dot"}

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "describe", "--flow", "missing.dot"])

    assert result == spark_cli.EXIT_NOT_FOUND
    assert "Unknown flow: missing.dot" in capsys.readouterr().err


def test_agent_trigger_create_and_delete_map_to_workspace_api(monkeypatch, tmp_path: Path, capsys) -> None:
    payload_path = tmp_path / "trigger.json"
    payload_path.write_text(
        json.dumps(
            {
                "name": "Daily build",
                "enabled": True,
                "source_type": "schedule",
                "source": {"kind": "interval", "interval_seconds": 60},
                "action": {"flow_name": TEST_AGENT_FLOW},
            }
        ),
        encoding="utf-8",
    )

    class FakeResponse:
        def __init__(self, payload: object) -> None:
            self.status_code = 200
            self.is_error = False
            self._payload = payload

        def json(self) -> object:
            return self._payload

    calls: list[tuple[str, str, dict[str, object] | None]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            calls.append((method, url, json))
            if method == "POST":
                return FakeResponse({"id": "trigger-123"})
            return FakeResponse({"status": "deleted", "id": "trigger-123"})

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    create_result = spark_cli.main(["trigger", "create", "--json", str(payload_path)])
    delete_result = spark_cli.main(["trigger", "delete", "--id", "trigger-123"])

    assert create_result == 0
    assert delete_result == 0
    assert calls == [
        (
            "POST",
            "http://127.0.0.1:8000/workspace/api/triggers",
            {
                "name": "Daily build",
                "enabled": True,
                "source_type": "schedule",
                "source": {"kind": "interval", "interval_seconds": 60},
                "action": {"flow_name": TEST_AGENT_FLOW},
            },
        ),
        (
            "DELETE",
            "http://127.0.0.1:8000/workspace/api/triggers/trigger-123",
            None,
        ),
    ]
    output = capsys.readouterr().out
    assert '"id": "trigger-123"' in output
