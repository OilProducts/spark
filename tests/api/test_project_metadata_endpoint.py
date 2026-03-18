from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import workspace.storage as workspace_storage


def _write_flow(name: str, content: str = "digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }\n") -> None:
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / name).write_text(content, encoding="utf-8")


def test_project_metadata_returns_name_directory_and_branch_for_git_repo(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    project_dir = tmp_path / "demo-project"
    project_dir.mkdir()

    branch_cmd = ["git", "-C", str(project_dir), "rev-parse", "--abbrev-ref", "HEAD"]
    commit_cmd = ["git", "-C", str(project_dir), "rev-parse", "HEAD"]

    def fake_run(cmd: list[str], capture_output: bool, text: bool, check: bool) -> subprocess.CompletedProcess[str]:
        assert cmd == branch_cmd or cmd == commit_cmd
        assert capture_output is True
        assert text is True
        assert check is True
        if cmd == branch_cmd:
            return subprocess.CompletedProcess(cmd, 0, stdout="feature/ui-metadata\n", stderr="")
        return subprocess.CompletedProcess(cmd, 0, stdout="abc123def456\n", stderr="")

    monkeypatch.setattr(server.subprocess, "run", fake_run)

    response = api_client.get(
        "/workspace/api/projects/metadata",
        params={"directory": str(project_dir)},
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "name": "demo-project",
        "directory": str(project_dir),
        "branch": "feature/ui-metadata",
        "commit": "abc123def456",
    }


def test_project_metadata_returns_null_branch_for_non_git_directory(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    project_dir = tmp_path / "non-git-project"
    project_dir.mkdir()

    def fake_run(cmd: list[str], capture_output: bool, text: bool, check: bool) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(returncode=128, cmd=cmd, stderr="not a git repository")

    monkeypatch.setattr(server.subprocess, "run", fake_run)

    response = api_client.get(
        "/workspace/api/projects/metadata",
        params={"directory": str(project_dir)},
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "name": "non-git-project",
        "directory": str(project_dir),
        "branch": None,
        "commit": None,
    }


def test_project_metadata_rejects_non_absolute_directory(api_client: TestClient) -> None:
    response = api_client.get(
        "/workspace/api/projects/metadata",
        params={"directory": "./relative-project"},
    )

    assert response.status_code == 400
    assert "must be absolute" in response.json()["detail"]


def test_project_directory_picker_returns_selected_absolute_directory(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "picked-project").resolve()
    project_dir.mkdir()

    monkeypatch.setattr(server, "_pick_project_directory", lambda prompt="": project_dir)

    response = api_client.post("/workspace/api/projects/pick-directory")

    assert response.status_code == 200
    assert response.json() == {
        "status": "selected",
        "directory_path": str(project_dir),
    }


def test_project_directory_picker_returns_canceled_when_no_directory_selected(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(server, "_pick_project_directory", lambda prompt="": None)

    response = api_client.post("/workspace/api/projects/pick-directory")

    assert response.status_code == 200
    assert response.json() == {"status": "canceled"}


def test_project_directory_picker_reports_unavailable_runtime(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fail(prompt: str = "") -> Path | None:
        raise RuntimeError("Native directory picker is unavailable.")

    monkeypatch.setattr(server, "_pick_project_directory", fail)

    response = api_client.post("/workspace/api/projects/pick-directory")

    assert response.status_code == 503
    assert response.json()["detail"] == "Native directory picker is unavailable."


def test_project_registry_endpoints_persist_project_metadata(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "registered-project").resolve()
    project_dir.mkdir()

    register_response = api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir)},
    )

    assert register_response.status_code == 200
    register_payload = register_response.json()
    assert register_payload["project_path"] == str(project_dir)
    assert register_payload["display_name"] == "registered-project"
    assert register_payload["is_favorite"] is False
    assert register_payload["active_conversation_id"] is None
    assert register_payload["flow_bindings"] == {}

    list_response = api_client.get("/workspace/api/projects")

    assert list_response.status_code == 200
    assert list_response.json() == [register_payload]

    project_file = server.get_settings().projects_dir / register_payload["project_id"] / "project.toml"
    assert project_file.exists()
    project_text = project_file.read_text(encoding="utf-8")
    assert f'project_path = "{project_dir}"' in project_text


def test_project_registry_logs_malformed_project_records(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    broken_project_dir = server.get_settings().projects_dir / "broken-project"
    broken_project_dir.mkdir(parents=True, exist_ok=True)
    broken_project_file = broken_project_dir / "project.toml"
    broken_project_file.write_text('project_path = "unterminated\n', encoding="utf-8")

    logged_messages: list[str] = []

    def fake_warning(message: str, *args: object) -> None:
        if args:
            logged_messages.append(message % args)
            return
        logged_messages.append(message)

    monkeypatch.setattr(workspace_storage.LOGGER, "warning", fake_warning)

    response = api_client.get("/workspace/api/projects")

    assert response.status_code == 200
    assert response.json() == []
    assert len(logged_messages) == 2
    assert f"Failed to read project record from {broken_project_file}:" in logged_messages[0]
    assert logged_messages[1] == f"Skipping project record with missing or invalid project_path in {broken_project_file}"


def test_project_state_endpoint_updates_favorite_and_conversation_metadata(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "tracked-project").resolve()
    project_dir.mkdir()

    api_client.post("/workspace/api/projects/register", json={"project_path": str(project_dir)})

    response = api_client.patch(
        "/workspace/api/projects/state",
        json={
            "project_path": str(project_dir),
            "is_favorite": True,
            "last_accessed_at": "2026-03-08T12:00:00Z",
            "active_conversation_id": "conversation-tracked-project",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["is_favorite"] is True
    assert payload["last_accessed_at"] == "2026-03-08T12:00:00Z"
    assert payload["active_conversation_id"] == "conversation-tracked-project"
    assert payload["flow_bindings"] == {}


def test_project_flow_binding_endpoints_persist_trigger_bindings(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "binding-project").resolve()
    project_dir.mkdir()
    _write_flow("plan-generation.dot")

    api_client.post("/workspace/api/projects/register", json={"project_path": str(project_dir)})

    put_response = api_client.put(
        "/workspace/api/projects/flow-bindings/spec_edit_approved",
        json={
            "project_path": str(project_dir),
            "flow_name": "plan-generation.dot",
        },
    )

    assert put_response.status_code == 200
    assert put_response.json() == {
        "project_path": str(project_dir),
        "flow_bindings": {
            "spec_edit_approved": "plan-generation.dot",
        },
    }

    get_response = api_client.get(
        "/workspace/api/projects/flow-bindings",
        params={"project_path": str(project_dir)},
    )

    assert get_response.status_code == 200
    assert get_response.json() == {
        "project_path": str(project_dir),
        "flow_bindings": {
            "spec_edit_approved": "plan-generation.dot",
        },
    }

    delete_response = api_client.delete(
        "/workspace/api/projects/flow-bindings/spec_edit_approved",
        params={"project_path": str(project_dir)},
    )

    assert delete_response.status_code == 200
    assert delete_response.json() == {
        "project_path": str(project_dir),
        "flow_bindings": {},
    }


def test_project_delete_endpoint_removes_registered_project_storage(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "delete-project").resolve()
    project_dir.mkdir()

    register_response = api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir)},
    )
    assert register_response.status_code == 200
    register_payload = register_response.json()

    project_root = server.get_settings().projects_dir / register_payload["project_id"]
    (project_root / "conversations" / "conversation-a").mkdir(parents=True)
    (project_root / "workflow").mkdir(parents=True, exist_ok=True)
    (project_root / "workflow" / "conversation-a.json").write_text("{}", encoding="utf-8")

    response = api_client.delete(
        "/workspace/api/projects",
        params={"project_path": str(project_dir)},
    )

    assert response.status_code == 200
    assert response.json() == {
        "status": "deleted",
        "project_id": register_payload["project_id"],
        "project_path": str(project_dir),
        "display_name": "delete-project",
    }
    assert not project_root.exists()

    list_response = api_client.get("/workspace/api/projects")
    assert list_response.status_code == 200
    assert list_response.json() == []
