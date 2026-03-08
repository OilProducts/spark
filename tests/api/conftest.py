from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server


@pytest.fixture(autouse=True)
def _reset_api_server_state(tmp_path: Path) -> None:
    server.configure_runtime_paths(
        data_dir=tmp_path / ".sparkspawn",
        runs_dir=tmp_path / "runs",
        flows_dir=tmp_path / "flows",
    )
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS.clear()
    server.HUMAN_BROKER = server.HumanGateBroker()
    server.EVENT_HUB = server.PipelineEventHub()
    server.RUNTIME.status = "idle"
    server.RUNTIME.last_error = ""
    server.RUNTIME.last_working_directory = ""
    server.RUNTIME.last_model = ""
    server.RUNTIME.last_completed_nodes = []
    server.RUNTIME.last_flow_name = ""
    server.RUNTIME.last_run_id = ""
    server.clear_registered_transforms()
    yield
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS.clear()
    server.clear_registered_transforms()


@pytest.fixture
def api_client() -> TestClient:
    with TestClient(server.app) as client:
        yield client
