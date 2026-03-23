from __future__ import annotations

from argparse import Namespace
import io
import json
from pathlib import Path

import spark.authoring_assets as authoring_assets
import spark.cli as spark_cli
import spark.starter_assets as starter_assets
import spark_server.cli as spark_server_cli


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
    assert calls == [
        {
            "app": "spark_app.app:app",
            "host": "127.0.0.1",
            "port": 8000,
            "reload": True,
        }
    ]


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


def test_run_init_seeds_missing_starter_flows_without_overwriting_existing(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(parents=True, exist_ok=True)
    existing_flow = flows_dir / "parallel-review.dot"
    existing_flow.write_text("custom-parallel\n", encoding="utf-8")

    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("parallel-review.dot", "canonical-parallel\n"),
            starter_assets.StarterFlowAsset("simple-linear.dot", "simple-linear\n"),
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
    assert (flows_dir / "simple-linear.dot").read_text(encoding="utf-8") == "simple-linear\n"
    output = capsys.readouterr().out
    assert "created=1 updated=0 skipped=1" in output


def test_run_init_force_overwrites_existing_starter_flows(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(parents=True, exist_ok=True)
    existing_flow = flows_dir / "parallel-review.dot"
    existing_flow.write_text("custom-parallel\n", encoding="utf-8")

    monkeypatch.setattr(
        starter_assets,
        "load_starter_flow_assets",
        lambda *, project_root=None: (
            starter_assets.StarterFlowAsset("parallel-review.dot", "canonical-parallel\n"),
            starter_assets.StarterFlowAsset("simple-linear.dot", "simple-linear\n"),
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
    assert (flows_dir / "simple-linear.dot").read_text(encoding="utf-8") == "simple-linear\n"
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
            starter_assets.StarterFlowAsset("supervision/implementation-worker.dot", "worker\n"),
            starter_assets.StarterFlowAsset("supervision/supervised-implementation.dot", "parent\n"),
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
    assert (flows_dir / "supervision" / "implementation-worker.dot").read_text(encoding="utf-8") == "worker\n"
    assert (flows_dir / "supervision" / "supervised-implementation.dot").read_text(encoding="utf-8") == "parent\n"
    output = capsys.readouterr().out
    assert "created=2 updated=0 skipped=0" in output


def test_packaged_starter_flows_match_repo_starter_flows() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    repo_starter_dir = repo_root / "starter-flows"
    packaged_starter_dir = repo_root / "src" / "spark" / "starter_flows"

    repo_payload = {
        path.relative_to(repo_starter_dir).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(repo_starter_dir.rglob("*.dot"))
    }
    packaged_payload = {
        path.relative_to(packaged_starter_dir).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(packaged_starter_dir.rglob("*.dot"))
    }

    assert packaged_payload == repo_payload


def test_packaged_authoring_references_match_repo_sources() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    packaged_guide = authoring_assets.dot_authoring_guide_path()
    packaged_attractor_spec = authoring_assets.attractor_spec_path()
    packaged_flow_extensions_spec = authoring_assets.flow_extensions_spec_path()

    assert packaged_guide.read_text(encoding="utf-8").startswith("# Spark DOT Authoring Guide")
    assert packaged_attractor_spec.read_text(encoding="utf-8") == (
        repo_root / "specs" / "attractor-spec.md"
    ).read_text(encoding="utf-8")
    assert packaged_flow_extensions_spec.read_text(encoding="utf-8") == (
        repo_root / "specs" / "spark-flow-extensions.md"
    ).read_text(encoding="utf-8")


def test_agent_spec_proposal_posts_payload_and_prints_response(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    payload_path = tmp_path / "proposal.json"
    payload_path.write_text(
        json.dumps(
            {
                "summary": "Clarify the approval gate.",
                "changes": [
                    {
                        "path": "specs/spark-workspace.md#proposal-review",
                        "before": "Planning begins immediately.",
                        "after": "Planning begins only after approval.",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> dict[str, object]:
            return {
                "ok": True,
                "conversation_id": "conversation-123",
                "conversation_handle": "amber-otter",
                "proposal_id": "proposal-123",
                "segment_id": "segment-artifact-proposal-123",
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
            "spec-proposal",
            "--conversation",
            "amber-otter",
            "--json",
            str(payload_path),
        ]
    )

    assert result == 0
    assert calls == [
        (
            "POST http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/spec-edit-proposals",
            {
                "summary": "Clarify the approval gate.",
                "changes": [
                    {
                        "path": "specs/spark-workspace.md#proposal-review",
                        "before": "Planning begins immediately.",
                        "after": "Planning begins only after approval.",
                    }
                ],
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["proposal_id"] == "proposal-123"


def test_agent_spec_proposal_rejects_payload_context_fields(tmp_path: Path, capsys) -> None:
    payload_path = tmp_path / "proposal.json"
    payload_path.write_text(
        json.dumps(
            {
                "summary": "Clarify approval.",
                "conversation_id": "conversation-123",
                "changes": [
                    {
                        "path": "specs/spark-workspace.md#proposal-review",
                        "before": "A",
                        "after": "B",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    result = spark_cli.main(
        ["convo", "spec-proposal", "--conversation", "amber-otter", "--json", str(payload_path)]
    )

    assert result == spark_cli.EXIT_GENERAL_FAILURE
    assert "Unexpected payload field" in capsys.readouterr().err


def test_agent_spec_proposal_reads_payload_from_stdin(monkeypatch, capsys) -> None:
    payload = {
        "summary": "Clarify the approval gate.",
        "changes": [
            {
                "path": "specs/spark-workspace.md#proposal-review",
                "before": "Planning begins immediately.",
                "after": "Planning begins only after approval.",
            }
        ],
        "rationale": "Ground the workflow in explicit approval.",
    }

    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> dict[str, object]:
            return {
                "ok": True,
                "conversation_id": "conversation-123",
                "conversation_handle": "quiet-river",
                "proposal_id": "proposal-123",
                "segment_id": "segment-artifact-proposal-123",
            }

    calls: list[dict[str, object]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: dict[str, object] | None = None):
            calls.append({"method": method, "url": url, "json": json})
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)
    monkeypatch.setattr(
        spark_cli.sys,
        "stdin",
        io.StringIO(json.dumps(payload)),
        raising=False,
    )

    result = spark_cli.main(["convo", "spec-proposal", "--conversation", "quiet-river", "--json", "-"])

    assert result == 0
    assert calls[0]["json"] == payload
    assert json.loads(capsys.readouterr().out)["proposal_id"] == "proposal-123"


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
            "implement-spec.dot",
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
                "flow_name": "implement-spec.dot",
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
            "implement-spec.dot",
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
                "flow_name": "implement-spec.dot",
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
            "implement-spec.dot",
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
                "flow_name": "implement-spec.dot",
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
                    "name": "implement-spec.dot",
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
    assert payload[0]["name"] == "implement-spec.dot"


def test_agent_describe_flow_text_mode_formats_human_readable_output(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return {
                "name": "implement-spec.dot",
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
            assert url == "http://127.0.0.1:8000/workspace/api/flows/implement-spec.dot?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "describe", "--flow", "implement-spec.dot", "--text"])

    assert result == 0
    output = capsys.readouterr().out
    assert "Name: implement-spec.dot" in output
    assert "Launch Policy: agent_requestable" in output


def test_agent_validate_flow_text_mode_formats_diagnostics(monkeypatch, capsys) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> object:
            return {
                "name": "implement-spec.dot",
                "path": "/flows/implement-spec.dot",
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
            assert url == "http://127.0.0.1:8000/workspace/api/flows/implement-spec.dot/validate"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "validate", "--flow", "implement-spec.dot", "--text"])

    assert result == 0
    output = capsys.readouterr().out
    assert "Status: invalid" in output
    assert "- ERROR missing-edge line 7: Missing edge." in output


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
            assert url == "http://127.0.0.1:8000/workspace/api/flows/implement-spec.dot/raw?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(spark_cli.httpx, "Client", FakeClient)

    result = spark_cli.main(["flow", "get", "--flow", "implement-spec.dot"])

    assert result == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["name"] == "implement-spec.dot"
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
                "action": {"flow_name": "implement-spec.dot"},
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
                "action": {"flow_name": "implement-spec.dot"},
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
