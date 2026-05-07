from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import attractor.api.server as server
import spark.app as product_app


def test_attractor_subapp_exposes_status_endpoint(attractor_api_client: TestClient) -> None:
    server.RUNTIME.status = "running"
    server.RUNTIME.last_flow_name = "test-dispatch.dot"

    response = attractor_api_client.get("/status")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "running"
    assert payload["last_flow_name"] == "test-dispatch.dot"
    assert {"last_error", "last_working_directory", "last_model", "last_completed_nodes"} <= set(payload)
    assert "last_run_id" not in payload


def test_workspace_subapp_exposes_project_listing(product_api_client: TestClient, tmp_path: Path) -> None:
    project_path = tmp_path / "workspace-project"
    project_path.mkdir(parents=True)

    register_response = product_api_client.post(
        "/workspace/api/projects/register",
        json={"project_path": str(project_path)},
    )
    assert register_response.status_code == 200

    response = product_api_client.get("/workspace/api/projects")

    assert response.status_code == 200
    payload = response.json()
    assert isinstance(payload, list)
    assert any(project["project_path"] == str(project_path) for project in payload)


def test_product_app_injects_attractor_runtime_paths() -> None:
    settings = product_app.get_settings()
    runtime_paths = server.get_runtime_paths()

    assert runtime_paths.runtime_dir == settings.runtime_dir
    assert runtime_paths.runs_dir == settings.runs_dir
    assert runtime_paths.flows_dir == settings.flows_dir


def test_root_app_serves_ui_index_and_static_assets(product_api_client: TestClient) -> None:
    index_response = product_api_client.get("/")
    favicon_response = product_api_client.get("/favicon.ico")
    spark_icon_response = product_api_client.get("/assets/spark-app-icon.png")

    assert index_response.status_code == 200
    assert index_response.headers["content-type"].startswith("text/html")
    assert favicon_response.status_code == 200
    assert favicon_response.headers["content-type"].startswith("image/png")
    assert spark_icon_response.status_code == 200


def test_root_app_does_not_expose_legacy_api_aliases(product_api_client: TestClient) -> None:
    assert product_api_client.get("/status").status_code == 404
    assert product_api_client.get("/runs").status_code == 404
    assert product_api_client.get("/api/projects").status_code == 404


def test_subapps_have_separate_docs_and_root_app_does_not(product_api_client: TestClient) -> None:
    assert product_api_client.get("/docs").status_code == 404
    assert product_api_client.get("/openapi.json").status_code == 404
    assert product_api_client.get("/attractor/docs").status_code == 200
    assert product_api_client.get("/workspace/docs").status_code == 200
