from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import spark.app as product_app
import spark.workspace.storage as workspace_storage


def _write_flow(name: str, content: str = "digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }\n") -> None:
    flows_dir = product_app.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / name).write_text(content, encoding="utf-8")


def _write_execution_profiles_config(content: str) -> None:
    config_dir = product_app.get_settings().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(content.strip(), encoding="utf-8")


def test_project_metadata_returns_name_directory_and_branch_for_git_repo(
    product_api_client: TestClient,
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

    monkeypatch.setattr(product_app.pipeline_runs.subprocess, "run", fake_run)

    response = product_api_client.get(
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
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    project_dir = tmp_path / "non-git-project"
    project_dir.mkdir()

    def fake_run(cmd: list[str], capture_output: bool, text: bool, check: bool) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(returncode=128, cmd=cmd, stderr="not a git repository")

    monkeypatch.setattr(product_app.pipeline_runs.subprocess, "run", fake_run)

    response = product_api_client.get(
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


def test_project_metadata_rejects_non_absolute_directory(product_api_client: TestClient) -> None:
    response = product_api_client.get(
        "/workspace/api/projects/metadata",
        params={"directory": "./relative-project"},
    )

    assert response.status_code == 400
    assert "must be absolute" in response.json()["detail"]


def test_project_directory_browser_defaults_to_service_user_home_directory(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    home_dir = (tmp_path / "service-home").resolve()
    home_dir.mkdir()
    monkeypatch.setenv("HOME", str(home_dir))

    response = product_api_client.get("/workspace/api/projects/browse")

    assert response.status_code == 200
    assert response.json() == {
        "current_path": str(home_dir),
        "parent_path": str(home_dir.parent),
        "roots": [],
        "entries": [],
    }


def test_project_directory_browser_defaults_to_first_configured_project_root(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    first_root = (tmp_path / "projects-a").resolve()
    second_root = (tmp_path / "projects-b").resolve()
    first_root.mkdir()
    second_root.mkdir()
    monkeypatch.setenv("SPARK_PROJECT_ROOTS", os.pathsep.join([str(first_root), str(second_root)]))
    product_app.configure_settings(data_dir=product_app.get_settings().data_dir, flows_dir=product_app.get_settings().flows_dir)

    response = product_api_client.get("/workspace/api/projects/browse")

    assert response.status_code == 200
    assert response.json() == {
        "current_path": str(first_root),
        "parent_path": str(first_root.parent),
        "roots": [str(first_root), str(second_root)],
        "entries": [],
    }


def test_project_directory_browser_supports_root_directory(product_api_client: TestClient) -> None:
    response = product_api_client.get("/workspace/api/projects/browse", params={"path": "/"})

    assert response.status_code == 200
    payload = response.json()
    assert payload["current_path"] == "/"
    assert payload["parent_path"] is None
    assert payload["roots"] == []
    assert all(entry["is_dir"] is True for entry in payload["entries"])


def test_project_directory_browser_returns_normalized_directory_only_entries(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "Browse Root").resolve()
    project_dir.mkdir()
    (project_dir / "zeta").mkdir()
    (project_dir / "Alpha").mkdir()
    (project_dir / ".hidden-dir").mkdir()
    (project_dir / "notes.txt").write_text("ignore me", encoding="utf-8")

    response = product_api_client.get(
        "/workspace/api/projects/browse",
        params={"path": str(project_dir / "nested" / "..")},
    )

    assert response.status_code == 200
    assert response.json() == {
        "current_path": str(project_dir),
        "parent_path": str(project_dir.parent),
        "roots": [],
        "entries": [
            {
                "name": ".hidden-dir",
                "path": str((project_dir / ".hidden-dir").resolve()),
                "is_dir": True,
            },
            {
                "name": "Alpha",
                "path": str((project_dir / "Alpha").resolve()),
                "is_dir": True,
            },
            {
                "name": "zeta",
                "path": str((project_dir / "zeta").resolve()),
                "is_dir": True,
            },
        ],
    }


def test_project_directory_browser_rejects_non_absolute_path(product_api_client: TestClient) -> None:
    response = product_api_client.get("/workspace/api/projects/browse", params={"path": "./relative-project"})

    assert response.status_code == 400
    assert response.json()["detail"] == "Browse path must be absolute."


def test_project_directory_browser_reports_missing_directory(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    missing_path = (tmp_path / "missing-project").resolve()

    response = product_api_client.get("/workspace/api/projects/browse", params={"path": str(missing_path)})

    assert response.status_code == 404
    assert response.json()["detail"] == f"Browse path does not exist: {missing_path}"


def test_project_directory_browser_rejects_file_paths(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    file_path = (tmp_path / "file.txt").resolve()
    file_path.write_text("not a directory", encoding="utf-8")

    response = product_api_client.get("/workspace/api/projects/browse", params={"path": str(file_path)})

    assert response.status_code == 400
    assert response.json()["detail"] == f"Browse path is not a directory: {file_path}"


def test_project_registry_endpoints_persist_project_metadata(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "registered-project").resolve()
    project_dir.mkdir()

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir)},
    )

    assert register_response.status_code == 200
    register_payload = register_response.json()
    assert register_payload["project_path"] == str(project_dir)
    assert register_payload["display_name"] == "registered-project"
    assert register_payload["is_favorite"] is False
    assert register_payload["active_conversation_id"] is None
    assert "flow_bindings" not in register_payload

    list_response = product_api_client.get("/workspace/api/projects")

    assert list_response.status_code == 200
    assert list_response.json() == [register_payload]

    project_file = product_app.get_settings().projects_dir / register_payload["project_id"] / "project.toml"
    assert project_file.exists()
    project_text = project_file.read_text(encoding="utf-8")
    assert f'project_path = "{project_dir}"' in project_text


def test_project_registry_persists_container_project_path_unchanged(product_api_client: TestClient) -> None:
    container_project_path = "/projects/my-app"

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": container_project_path},
    )

    assert register_response.status_code == 200
    register_payload = register_response.json()
    assert register_payload["project_path"] == container_project_path

    project_file = product_app.get_settings().projects_dir / register_payload["project_id"] / "project.toml"
    assert project_file.exists()
    assert f'project_path = "{container_project_path}"' in project_file.read_text(encoding="utf-8")


def test_project_registry_persists_execution_profile_default(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "container-default").resolve()
    project_dir.mkdir()
    _write_execution_profiles_config(
        """
        [profiles.local-dev]
        label = "Local Dev"
        mode = "local_container"
        image = "spark-exec:latest"

        [profiles.remote-fast]
        label = "Remote Fast"
        mode = "native"
        """
    )

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir), "execution_profile_id": "local-dev"},
    )
    assert register_response.status_code == 200
    assert register_response.json()["execution_profile_id"] == "local-dev"

    update_response = product_api_client.patch(
        "/workspace/api/projects/state",
        json={"project_path": str(project_dir), "execution_profile_id": "remote-fast"},
    )
    assert update_response.status_code == 200
    assert update_response.json()["execution_profile_id"] == "remote-fast"

    project_file = product_app.get_settings().projects_dir / update_response.json()["project_id"] / "project.toml"
    assert 'execution_profile_id = "remote-fast"' in project_file.read_text(encoding="utf-8")


def test_project_registry_rejects_unknown_execution_profile_default(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "unknown-default").resolve()
    project_dir.mkdir()
    _write_execution_profiles_config(
        """
        [profiles.local-dev]
        label = "Local Dev"
        mode = "native"
        """
    )

    response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir), "execution_profile_id": "missing-profile"},
    )

    assert response.status_code == 400
    assert response.json()["detail"] == "Unknown execution profile: missing-profile"


def test_project_registry_rejects_disabled_execution_profile_default(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "disabled-default").resolve()
    project_dir.mkdir()
    _write_execution_profiles_config(
        """
        [profiles.disabled-local]
        label = "Disabled Local"
        mode = "local_container"
        enabled = false
        image = "spark-exec:latest"
        """
    )

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir)},
    )
    assert register_response.status_code == 200

    response = product_api_client.patch(
        "/workspace/api/projects/state",
        json={"project_path": str(project_dir), "execution_profile_id": "disabled-local"},
    )

    assert response.status_code == 400
    assert response.json()["detail"] == "Execution profile is disabled: disabled-local"


def test_project_registry_logs_malformed_project_records(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    broken_project_dir = product_app.get_settings().projects_dir / "broken-project"
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

    response = product_api_client.get("/workspace/api/projects")

    assert response.status_code == 200
    assert response.json() == []
    assert len(logged_messages) == 2
    assert f"Failed to read project record from {broken_project_file}:" in logged_messages[0]
    assert logged_messages[1] == f"Skipping project record with missing or invalid project_path in {broken_project_file}"


def test_project_state_endpoint_updates_favorite_and_conversation_metadata(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "tracked-project").resolve()
    project_dir.mkdir()

    product_api_client.post("/workspace/api/projects/register", json={"project_path": str(project_dir)})

    response = product_api_client.patch(
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
    assert "flow_bindings" not in payload


def test_project_delete_endpoint_removes_registered_project_storage(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "delete-project").resolve()
    project_dir.mkdir()

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_dir)},
    )
    assert register_response.status_code == 200
    register_payload = register_response.json()

    project_root = product_app.get_settings().projects_dir / register_payload["project_id"]
    (project_root / "conversations" / "conversation-a").mkdir(parents=True)
    (project_root / "workflow").mkdir(parents=True, exist_ok=True)
    (project_root / "workflow" / "conversation-a.json").write_text("{}", encoding="utf-8")

    response = product_api_client.delete(
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

    list_response = product_api_client.get("/workspace/api/projects")
    assert list_response.status_code == 200
    assert list_response.json() == []
