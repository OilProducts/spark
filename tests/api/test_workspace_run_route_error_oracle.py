from __future__ import annotations

from pathlib import Path

import spark.app as product_app


def test_workspace_run_routes_preserve_fastapi_validation_envelopes(product_api_client) -> None:
    missing_launch_field = product_api_client.post(
        "/workspace/api/runs/launch",
        json={"summary": "Missing flow_name.", "project_path": "/tmp/project"},
    )
    assert missing_launch_field.status_code == 422
    assert missing_launch_field.json()["detail"][0]["type"] == "missing"
    assert missing_launch_field.json()["detail"][0]["loc"] == ["body", "flow_name"]

    retry_extra = product_api_client.post(
        "/workspace/api/runs/run-1/retry",
        json={"conversation_handle": "amber-anchor", "extra": True},
    )
    assert retry_extra.status_code == 422
    assert retry_extra.json()["detail"][0]["type"] == "extra_forbidden"
    assert retry_extra.json()["detail"][0]["loc"] == ["body", "extra"]

    continue_missing_mode = product_api_client.post(
        "/workspace/api/runs/run-1/continue",
        json={"start_node": "task"},
    )
    assert continue_missing_mode.status_code == 422
    assert continue_missing_mode.json()["detail"][0]["type"] == "missing"
    assert continue_missing_mode.json()["detail"][0]["loc"] == ["body", "flow_source_mode"]

    malformed_retry = product_api_client.post(
        "/workspace/api/runs/run-1/retry",
        data="{",
        headers={"content-type": "application/json"},
    )
    assert malformed_retry.status_code == 422
    assert malformed_retry.json()["detail"][0]["type"] == "json_invalid"
    assert malformed_retry.json()["detail"][0]["loc"][0] == "body"


def test_workspace_direct_launch_invalid_dot_returns_http_500_parse_detail(
    product_api_client, tmp_path: Path
) -> None:
    settings = product_app.get_settings()
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    flow_path = settings.flows_dir / "broken-launch.dot"
    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text("digraph Broken { start -> }\n", encoding="utf-8")

    response = product_api_client.post(
        "/workspace/api/runs/launch",
        json={
            "flow_name": "broken-launch.dot",
            "summary": "Launch an invalid flow.",
            "project_path": str(project_dir),
        },
    )

    assert response.status_code == 500
    detail = response.json()["detail"]
    assert isinstance(detail, str)
    assert detail
    assert detail != "Flow launch could not be started."
