from __future__ import annotations

from textwrap import dedent

from fastapi.testclient import TestClient

import spark.app as product_app


def test_attractor_execution_placement_settings_exposes_native_and_local_profiles_only(
    product_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "execution-profiles.toml").write_text(
        dedent(
            """
            [defaults]
            execution_profile_id = "local-dev"

            [workers.worker-a]
            label = "Ignored Worker"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "WORKER_A_TOKEN"

            [profiles.native-dev]
            label = "Native Dev"
            mode = "native"

            [profiles.local-dev]
            label = "Local Dev"
            mode = "local_container"
            image = "spark-exec:latest"
            """
        ).strip(),
        encoding="utf-8",
    )

    response = product_api_client.get("/attractor/api/execution-placement-settings")

    assert response.status_code == 200
    payload = response.json()
    assert payload["execution_modes"] == ["native", "local_container"]
    assert payload["default_execution_profile_id"] == "local-dev"
    assert payload["validation_errors"] == []
    assert "workers" not in payload
    assert "protocol" not in payload
    assert {profile["mode"] for profile in payload["profiles"]} == {
        "native",
        "local_container",
    }
    local_profile = next(profile for profile in payload["profiles"] if profile["id"] == "local-dev")
    assert local_profile["image"] == "spark-exec:latest"


def test_workspace_settings_wraps_execution_placement_payload(product_api_client: TestClient) -> None:
    response = product_api_client.get("/workspace/api/settings")

    assert response.status_code == 200
    payload = response.json()
    execution_placement = payload["execution_placement"]
    assert execution_placement["execution_modes"] == ["native", "local_container"]
    assert execution_placement["profiles"] == [
        {
            "id": "native",
            "label": "Native",
            "mode": "native",
            "enabled": True,
            "image": None,
            "capabilities": {},
            "metadata": {},
        }
    ]
    assert "workers" not in execution_placement
    assert "protocol" not in execution_placement
    assert execution_placement["config"]["synthesized_native_default"] is True


def test_execution_placement_settings_returns_validation_errors_for_removed_remote_worker_mode(
    product_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    (settings.config_dir / "execution-profiles.toml").write_text(
        dedent(
            """
            [profiles.remote]
            label = "Remote"
            mode = "remote_worker"
            image = "spark-worker:latest"
            """
        ).strip(),
        encoding="utf-8",
    )

    response = product_api_client.get("/workspace/api/settings")

    assert response.status_code == 200
    execution_placement = response.json()["execution_placement"]
    assert execution_placement["execution_modes"] == ["native", "local_container"]
    assert execution_placement["profiles"] == []
    assert "workers" not in execution_placement
    assert execution_placement["config"]["loaded"] is False
    assert execution_placement["validation_errors"] == [
        {
            "field": "profiles.remote.mode",
            "message": "execution mode must be one of: native, local_container",
            "profile_id": "remote",
        }
    ]
