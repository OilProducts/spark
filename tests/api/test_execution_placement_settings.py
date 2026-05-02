from __future__ import annotations

from pathlib import Path
from textwrap import dedent

from fastapi.testclient import TestClient

import attractor.execution.settings_view as settings_view
import spark.app as product_app
from attractor.execution.worker_models import WorkerHealthResponse, WorkerInfoResponse


def test_attractor_execution_placement_settings_exposes_profiles_workers_and_worker_metadata(
    product_api_client: TestClient,
    monkeypatch,
    tmp_path: Path,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "execution-profiles.toml").write_text(
        dedent(
            f"""
            [defaults]
            execution_profile_id = "remote-fast"

            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "WORKER_A_TOKEN"
            capabilities = ["containers", "node-events"]

            [profiles.native-dev]
            label = "Native Dev"
            mode = "native"

            [profiles.local-dev]
            label = "Local Dev"
            mode = "local_container"
            image = "spark-exec:latest"

            [profiles.remote-fast]
            label = "Remote Fast"
            mode = "remote_worker"
            worker = "worker-a"
            image = "spark-worker:latest"
            control_project_root = "{tmp_path}"
            worker_project_root = "/workspace/project"
            worker_runtime_root = "/workspace/runtime"
            capabilities = ["containers"]
            """
        ).strip(),
        encoding="utf-8",
    )
    monkeypatch.setattr(settings_view, "RemoteWorkerClient", _WorkerSettingsClient)

    response = product_api_client.get("/attractor/api/execution-placement-settings")

    assert response.status_code == 200
    payload = response.json()
    assert payload["execution_modes"] == ["native", "local_container", "remote_worker"]
    assert payload["default_execution_profile_id"] == "remote-fast"
    assert payload["validation_errors"] == []
    assert {profile["mode"] for profile in payload["profiles"]} == {
        "native",
        "local_container",
        "remote_worker",
    }
    remote_profile = next(profile for profile in payload["profiles"] if profile["id"] == "remote-fast")
    assert remote_profile["worker_id"] == "worker-a"
    assert remote_profile["image"] == "spark-worker:latest"

    worker = payload["workers"][0]
    assert worker["id"] == "worker-a"
    assert worker["enabled"] is True
    assert worker["health"]["status"] == "ready"
    assert worker["worker_info"]["supported_images"] == ["spark-worker:latest"]
    assert worker["versions"] == {
        "worker_version": "1.2.3",
        "protocol_version": "v1",
        "expected_protocol_version": "v1",
    }
    assert worker["protocol_compatible"] is True


def test_workspace_settings_wraps_execution_placement_payload(product_api_client: TestClient) -> None:
    response = product_api_client.get("/workspace/api/settings")

    assert response.status_code == 200
    payload = response.json()
    execution_placement = payload["execution_placement"]
    assert execution_placement["execution_modes"] == ["native", "local_container", "remote_worker"]
    assert execution_placement["profiles"] == [
        {
            "id": "native",
            "label": "Native",
            "mode": "native",
            "enabled": True,
            "worker_id": None,
            "image": None,
            "control_project_root": None,
            "worker_project_root": None,
            "worker_runtime_root": None,
            "capabilities": {},
            "metadata": {},
        }
    ]
    assert execution_placement["workers"] == []
    assert execution_placement["config"]["synthesized_native_default"] is True


def test_execution_placement_settings_returns_validation_errors_without_legacy_container_mode(
    product_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "execution-profiles.toml").write_text(
        dedent(
            """
            [profiles.legacy]
            label = "Legacy"
            mode = "container"
            image = "spark-exec:latest"
            """
        ).strip(),
        encoding="utf-8",
    )

    response = product_api_client.get("/workspace/api/settings")

    assert response.status_code == 200
    execution_placement = response.json()["execution_placement"]
    assert execution_placement["execution_modes"] == ["native", "local_container", "remote_worker"]
    assert execution_placement["profiles"] == []
    assert execution_placement["workers"] == []
    assert execution_placement["config"]["loaded"] is False
    assert execution_placement["validation_errors"] == [
        {
            "field": "profiles.legacy.mode",
            "message": "execution mode must be one of: native, local_container, remote_worker",
            "profile_id": "legacy",
            "worker_id": None,
        }
    ]


class _WorkerSettingsClient:
    def __init__(self, _worker):
        pass

    def __enter__(self):
        return self

    def __exit__(self, *_exc_info):
        return None

    def health(self) -> WorkerHealthResponse:
        return WorkerHealthResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ready",
            capabilities={"containers": True},
        )

    def worker_info(self) -> WorkerInfoResponse:
        return WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ready",
            capabilities={"containers": True},
            supported_images=["spark-worker:latest"],
            version_details={"runtime": "test"},
        )
