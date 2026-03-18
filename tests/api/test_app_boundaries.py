from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import attractor.api.server as server


def test_attractor_subapp_exposes_status_endpoint(api_client: TestClient) -> None:
    server.RUNTIME.status = "running"
    server.RUNTIME.last_flow_name = "implement-spec.dot"
    server.RUNTIME.last_run_id = "run-123"

    response = api_client.get("/attractor/status")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "running"
    assert payload["last_flow_name"] == "implement-spec.dot"
    assert payload["last_run_id"] == "run-123"
    assert {"last_error", "last_working_directory", "last_model", "last_completed_nodes"} <= set(payload)


def test_workspace_subapp_exposes_project_listing(api_client: TestClient, tmp_path: Path) -> None:
    project_path = tmp_path / "workspace-project"
    project_path.mkdir(parents=True)

    register_response = api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_path)},
    )
    assert register_response.status_code == 200

    response = api_client.get("/workspace/api/projects")

    assert response.status_code == 200
    payload = response.json()
    assert isinstance(payload, list)
    assert any(project["project_path"] == str(project_path) for project in payload)


def test_root_app_does_not_expose_legacy_api_aliases(api_client: TestClient) -> None:
    assert api_client.get("/status").status_code == 404
    assert api_client.get("/runs").status_code == 404
    assert api_client.get("/api/projects").status_code == 404


def test_subapps_have_separate_docs_and_root_app_does_not(api_client: TestClient) -> None:
    assert api_client.get("/docs").status_code == 404
    assert api_client.get("/openapi.json").status_code == 404
    assert api_client.get("/attractor/docs").status_code == 200
    assert api_client.get("/workspace/docs").status_code == 200
