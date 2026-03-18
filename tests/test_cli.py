from __future__ import annotations

from argparse import Namespace
import json
from pathlib import Path
import io

import attractor.cli as cli


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

    result = cli._run_serve(args)

    assert result == 0
    assert calls == [
        {
            "app": "attractor.api.server:app",
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

    cli._run_serve(args)

    assert cli.os.environ["SPARKSPAWN_HOME"] == str(data_dir.resolve(strict=False))
    assert cli.os.environ["SPARKSPAWN_FLOWS_DIR"] == str(flows_dir.resolve(strict=False))
    assert cli.os.environ["SPARKSPAWN_UI_DIR"] == str(ui_dir.resolve(strict=False))


def test_run_workspace_spec_proposal_posts_payload_and_prints_response(
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
                        "path": "specs/sparkspawn-workspace.md#proposal-review",
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

        def post(self, url: str, json: dict[str, object]) -> FakeResponse:
            calls.append((url, json))
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(
        [
            "spec-proposal",
            "--conversation",
            "amber-otter",
            "--json",
            str(payload_path),
            "--base-url",
            "http://127.0.0.1:8000",
        ]
    )

    assert result == 0
    assert calls == [
        (
            "http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/spec-edit-proposals",
            {
                "summary": "Clarify the approval gate.",
                "changes": [
                    {
                        "path": "specs/sparkspawn-workspace.md#proposal-review",
                        "before": "Planning begins immediately.",
                        "after": "Planning begins only after approval.",
                    }
                ],
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["proposal_id"] == "proposal-123"


def test_run_workspace_spec_proposal_rejects_payload_context_fields(
    tmp_path: Path,
    capsys,
) -> None:
    payload_path = tmp_path / "proposal.json"
    payload_path.write_text(
        json.dumps(
            {
                "conversation_id": "conversation-123",
                "summary": "Wrong payload shape",
                "changes": [],
            }
        ),
        encoding="utf-8",
    )

    result = cli.workspace_main(["spec-proposal", "--conversation", "amber-otter", "--json", str(payload_path)])

    assert result == 1
    stderr = json.loads(capsys.readouterr().err)
    assert stderr["ok"] is False
    assert "--conversation" in stderr["error"]


def test_run_workspace_spec_proposal_reads_payload_from_stdin(
    monkeypatch,
    capsys,
) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False

        def json(self) -> dict[str, object]:
            return {
                "ok": True,
                "conversation_id": "conversation-stdin",
                "conversation_handle": "quiet-river",
                "proposal_id": "proposal-stdin",
                "segment_id": "segment-artifact-proposal-stdin",
            }

    calls: list[tuple[str, dict[str, object]]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def post(self, url: str, json: dict[str, object]) -> FakeResponse:
            calls.append((url, json))
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)
    monkeypatch.setattr(
        cli.sys,
        "stdin",
        io.StringIO(
            json.dumps(
                {
                    "summary": "Read from stdin.",
                    "changes": [
                        {
                            "path": "specs/sparkspawn-workspace.md#proposal-review",
                            "before": "Old text.",
                            "after": "New text.",
                        }
                    ],
                }
            )
        ),
    )

    result = cli.workspace_main(["spec-proposal", "--conversation", "quiet-river", "--json", "-"])

    assert result == 0
    assert calls == [
        (
            "http://127.0.0.1:8000/workspace/api/conversations/by-handle/quiet-river/spec-edit-proposals",
            {
                "summary": "Read from stdin.",
                "changes": [
                    {
                        "path": "specs/sparkspawn-workspace.md#proposal-review",
                        "before": "Old text.",
                        "after": "New text.",
                    }
                ],
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["proposal_id"] == "proposal-stdin"


def test_run_workspace_flow_run_posts_payload_and_prints_response(
    monkeypatch,
    tmp_path: Path,
    capsys,
) -> None:
    goal_path = tmp_path / "goal.txt"
    goal_path.write_text("Implement the approved scope.", encoding="utf-8")

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

        def post(self, url: str, json: dict[str, object]) -> FakeResponse:
            calls.append((url, json))
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(
        [
            "flow-run",
            "--conversation",
            "amber-otter",
            "--flow",
            "implement-spec.dot",
            "--summary",
            "Run implementation for the approved scope",
            "--goal-file",
            str(goal_path),
            "--model",
            "gpt-5.4",
            "--base-url",
            "http://127.0.0.1:8000",
        ]
    )

    assert result == 0
    assert calls == [
        (
            "http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/flow-run-requests",
            {
                "flow_name": "implement-spec.dot",
                "summary": "Run implementation for the approved scope",
                "goal": "Implement the approved scope.",
                "model": "gpt-5.4",
            },
        )
    ]
    assert json.loads(capsys.readouterr().out)["flow_run_request_id"] == "flow-run-request-123"


def test_workspace_list_flows_defaults_to_json(
    monkeypatch,
    capsys,
) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False
        text = ""

        def json(self) -> list[dict[str, object]]:
            return [
                {
                    "name": "implement-spec.dot",
                    "title": "Implement Spec",
                    "description": "Execute an approved plan.",
                }
            ]

    calls: list[tuple[str, str, object | None]] = []

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: object | None = None) -> FakeResponse:
            calls.append((method, url, json))
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(["list-flows", "--base-url", "http://127.0.0.1:8000"])

    assert result == 0
    assert calls == [("GET", "http://127.0.0.1:8000/workspace/api/flows?surface=agent", None)]
    assert json.loads(capsys.readouterr().out) == [
        {
            "description": "Execute an approved plan.",
            "name": "implement-spec.dot",
            "title": "Implement Spec",
        }
    ]


def test_workspace_describe_flow_text_mode_formats_human_readable_output(
    monkeypatch,
    capsys,
) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False
        text = ""

        def json(self) -> dict[str, object]:
            return {
                "name": "implement-spec.dot",
                "title": "Implement Spec",
                "description": "Execute an approved plan.",
                "effective_launch_policy": "agent_requestable",
                "graph_label": "Implement Spec",
                "graph_goal": "Execute plan",
                "node_count": 4,
                "edge_count": 3,
                "features": {
                    "has_human_gate": False,
                    "has_manager_loop": True,
                },
            }

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: object | None = None) -> FakeResponse:
            assert method == "GET"
            assert json is None
            assert url == "http://127.0.0.1:8000/workspace/api/flows/implement-spec.dot?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(
        [
            "describe-flow",
            "--flow",
            "implement-spec.dot",
            "--text",
            "--base-url",
            "http://127.0.0.1:8000",
        ]
    )

    assert result == 0
    output = capsys.readouterr().out
    assert "Name: implement-spec.dot" in output
    assert "Title: Implement Spec" in output
    assert "Launch Policy: agent_requestable" in output
    assert "Has Manager Loop: True" in output


def test_workspace_get_flow_defaults_to_json_wrapper(
    monkeypatch,
    capsys,
) -> None:
    class FakeResponse:
        status_code = 200
        is_error = False
        text = 'digraph G { start -> done; }\n'

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str) -> FakeResponse:
            assert method == "GET"
            assert url == "http://127.0.0.1:8000/workspace/api/flows/implement-spec.dot/raw?surface=agent"
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(
        [
            "get-flow",
            "--flow",
            "implement-spec.dot",
            "--base-url",
            "http://127.0.0.1:8000",
        ]
    )

    assert result == 0
    assert json.loads(capsys.readouterr().out) == {
        "content": 'digraph G { start -> done; }\n',
        "name": "implement-spec.dot",
    }


def test_workspace_flow_discovery_returns_not_found_exit_code_on_404(
    monkeypatch,
    capsys,
) -> None:
    class FakeResponse:
        status_code = 404
        is_error = True
        text = '{"detail":"Unknown flow: missing.dot"}'

        def json(self) -> dict[str, object]:
            return {"detail": "Unknown flow: missing.dot"}

    class FakeClient:
        def __init__(self, timeout: float) -> None:
            assert timeout == 30.0

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb) -> None:
            return None

        def request(self, method: str, url: str, json: object | None = None) -> FakeResponse:
            assert method == "GET"
            assert json is None
            return FakeResponse()

    monkeypatch.setattr(cli.httpx, "Client", FakeClient)

    result = cli.workspace_main(["describe-flow", "--flow", "missing.dot"])

    assert result == cli.EXIT_NOT_FOUND
    stderr = json.loads(capsys.readouterr().err)
    assert stderr == {
        "ok": False,
        "status_code": 404,
        "error": "Unknown flow: missing.dot",
    }
