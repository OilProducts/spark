from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_terminal_status as _wait_for_pipeline_terminal_status,
)


def test_cancel_pipeline_returns_404_for_unknown_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = api_client.post("/attractor/pipelines/missing-run/cancel")
    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_cancel_pipeline_requests_cancel_for_active_run(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    response = api_client.post(f"/attractor/pipelines/{run_id}/cancel")
    assert response.status_code == 200
    payload = response.json()

    status_response = api_client.get(f"/attractor/pipelines/{run_id}")
    assert status_response.status_code == 200
    status_payload = status_response.json()

    runs_response = api_client.get("/attractor/runs")
    assert runs_response.status_code == 200
    run_rows = runs_response.json()["runs"]
    row = next((entry for entry in run_rows if entry["run_id"] == run_id), None)
    assert row is not None
    assert row["status"] == "cancel_requested"

    assert payload == {"status": "cancel_requested", "pipeline_id": run_id}
    assert status_payload["status"] == "cancel_requested"
    assert status_payload["last_error"] == "cancel_requested_by_user"
    assert server.RUNTIME.status == "cancel_requested"
    assert server.RUNTIME.last_error == "cancel_requested_by_user"

    history = server.EVENT_HUB.history(run_id)
    assert {"type": "runtime", "status": "cancel_requested", "run_id": run_id} in history
    assert {
        "type": "log",
        "msg": "[System] Cancel requested. Stopping after current node.",
        "run_id": run_id,
    } in history


def test_cancel_pipeline_ignores_non_running_known_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    final_status = _wait_for_pipeline_terminal_status(api_client, run_id)
    assert final_status == "success"

    response = api_client.post(f"/attractor/pipelines/{run_id}/cancel")
    assert response.status_code == 200
    payload = response.json()

    assert payload == {"status": "ignored", "pipeline_id": run_id}
