from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import spark_app.app as product_app


@pytest.fixture(autouse=True)
def _reset_api_server_state(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.delenv("SPARK_UI_DIR", raising=False)
    server.shutdown_attractor_runtime()
    server.configure_runtime_paths(
        data_dir=tmp_path / ".spark",
        runs_dir=None,
        flows_dir=tmp_path / "flows",
        ui_dir=None,
    )
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS.clear()
    server.HUMAN_BROKER = server.HumanGateBroker()
    server.EVENT_HUB = server.PipelineEventHub()
    server.RUNTIME.status = "idle"
    server.RUNTIME.outcome = None
    server.RUNTIME.outcome_reason_code = None
    server.RUNTIME.outcome_reason_message = None
    server.RUNTIME.last_error = ""
    server.RUNTIME.last_working_directory = ""
    server.RUNTIME.last_model = ""
    server.RUNTIME.last_completed_nodes = []
    server.RUNTIME.last_flow_name = ""
    server.RUNTIME.last_run_id = ""
    server.clear_registered_transforms()
    yield
    server.shutdown_attractor_runtime()
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS.clear()
    server.clear_registered_transforms()


@pytest.fixture
def attractor_api_client() -> TestClient:
    with TestClient(server.attractor_app) as client:
        yield client


@pytest.fixture
def product_api_client() -> TestClient:
    with TestClient(product_app.app) as client:
        yield client
