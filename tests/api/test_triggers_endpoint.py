from __future__ import annotations

import asyncio
from pathlib import Path
import time
from types import SimpleNamespace

import pytest
from fastapi.testclient import TestClient

import spark.app as product_app
from spark.workspace.triggers import TriggerRuntime, create_trigger_definition
from tests.support.flow_fixtures import seed_flow_fixture

TEST_PLANNING_FLOW = "test-planning.dot"


def _seed_flow(name: str) -> None:
    seed_flow_fixture(product_app.get_settings().flows_dir, "minimal-valid.dot", as_name=name)


def test_create_and_list_custom_schedule_trigger(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "schedule-project").resolve()
    project_dir.mkdir()
    _seed_flow(TEST_PLANNING_FLOW)

    create_response = product_api_client.post(
        "/workspace/api/triggers",
        json={
            "name": "Daily planning",
            "enabled": True,
            "source_type": "schedule",
            "action": {
                "flow_name": TEST_PLANNING_FLOW,
                "project_path": str(project_dir),
                "static_context": {"origin": "test"},
            },
            "source": {
                "kind": "interval",
                "interval_seconds": 300,
            },
        },
    )

    assert create_response.status_code == 200
    created_payload = create_response.json()
    assert created_payload["name"] == "Daily planning"
    assert created_payload["source_type"] == "schedule"
    assert created_payload["action"]["flow_name"] == TEST_PLANNING_FLOW
    assert created_payload["action"]["project_path"] == str(project_dir)
    assert created_payload["state"]["next_run_at"]

    list_response = product_api_client.get("/workspace/api/triggers")
    assert list_response.status_code == 200
    listed = list_response.json()
    assert [entry["id"] for entry in listed] == [created_payload["id"]]


def test_webhook_trigger_accepts_valid_secret_and_launches_flow(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "webhook-project").resolve()
    project_dir.mkdir()
    _seed_flow("webhook.dot")

    create_response = product_api_client.post(
        "/workspace/api/triggers",
        json={
            "name": "Webhook launch",
            "enabled": True,
            "source_type": "webhook",
            "action": {
                "flow_name": "webhook.dot",
                "project_path": str(project_dir),
                "static_context": {"source": "webhook-test"},
            },
            "source": {},
        },
    )

    assert create_response.status_code == 200
    trigger_payload = create_response.json()
    webhook_key = trigger_payload["source"]["webhook_key"]
    webhook_secret = trigger_payload["webhook_secret"]

    bad_secret_response = product_api_client.post(
        "/workspace/api/webhooks",
        headers={
            "X-Spark-Webhook-Key": webhook_key,
            "X-Spark-Webhook-Secret": "wrong-secret",
        },
        json={"value": 1},
    )
    assert bad_secret_response.status_code == 403

    webhook_response = product_api_client.post(
        "/workspace/api/webhooks",
        headers={
            "X-Spark-Webhook-Key": webhook_key,
            "X-Spark-Webhook-Secret": webhook_secret,
            "X-Spark-Webhook-Request-Id": "request-1",
        },
        json={"value": 1},
    )
    assert webhook_response.status_code == 200
    assert webhook_response.json()["trigger_id"] == trigger_payload["id"]

    deadline = time.time() + 5.0
    while time.time() < deadline:
        runs_response = product_api_client.get("/attractor/runs", params={"project_path": str(project_dir)})
        assert runs_response.status_code == 200
        run_ids = [run["run_id"] for run in runs_response.json()["runs"] if run["flow_name"] == "webhook.dot"]
        if run_ids:
            break
        time.sleep(0.1)
    else:
        pytest.fail("Timed out waiting for webhook trigger run.")


def test_webhook_trigger_repeats_launches_for_duplicate_request_ids(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    project_dir = (tmp_path / "webhook-duplicate-project").resolve()
    project_dir.mkdir()
    _seed_flow("webhook-duplicate.dot")

    create_response = product_api_client.post(
        "/workspace/api/triggers",
        json={
            "name": "Webhook duplicate launch",
            "enabled": True,
            "source_type": "webhook",
            "action": {
                "flow_name": "webhook-duplicate.dot",
                "project_path": str(project_dir),
                "static_context": {"source": "webhook-test"},
            },
            "source": {},
        },
    )

    assert create_response.status_code == 200
    trigger_payload = create_response.json()
    webhook_key = trigger_payload["source"]["webhook_key"]
    webhook_secret = trigger_payload["webhook_secret"]

    for _ in range(2):
        webhook_response = product_api_client.post(
            "/workspace/api/webhooks",
            headers={
                "X-Spark-Webhook-Key": webhook_key,
                "X-Spark-Webhook-Secret": webhook_secret,
                "X-Spark-Webhook-Request-Id": "request-1",
            },
            json={"value": 1},
        )
        assert webhook_response.status_code == 200

    deadline = time.time() + 5.0
    while time.time() < deadline:
        runs_response = product_api_client.get("/attractor/runs", params={"project_path": str(project_dir)})
        assert runs_response.status_code == 200
        run_ids = [run["run_id"] for run in runs_response.json()["runs"] if run["flow_name"] == "webhook-duplicate.dot"]
        if len(run_ids) >= 2:
            break
        time.sleep(0.1)
    else:
        pytest.fail("Timed out waiting for duplicate webhook trigger runs.")


@pytest.mark.asyncio
async def test_trigger_runtime_no_longer_dedupes_flow_events_or_poll_items(tmp_path: Path) -> None:
    settings = SimpleNamespace(
        config_dir=(tmp_path / "config").resolve(),
        data_dir=(tmp_path / "data").resolve(),
    )
    settings.config_dir.mkdir(parents=True, exist_ok=True)
    settings.data_dir.mkdir(parents=True, exist_ok=True)

    create_trigger_definition(
        settings.config_dir,
        name="Flow event launch",
        enabled=True,
        source_type="flow_event",
        action={
            "flow_name": TEST_PLANNING_FLOW,
            "project_path": str((tmp_path / "project").resolve()),
            "static_context": {},
        },
        source={
            "flow_name": "upstream.dot",
            "statuses": ["completed"],
        },
    )
    create_trigger_definition(
        settings.config_dir,
        name="Poll launch",
        enabled=True,
        source_type="poll",
        action={
            "flow_name": TEST_PLANNING_FLOW,
            "project_path": str((tmp_path / "project").resolve()),
            "static_context": {},
        },
        source={
            "url": "https://example.test/items",
            "interval_seconds": 1,
            "items_path": "items",
            "item_id_path": "id",
        },
    )

    runtime = TriggerRuntime(
        get_settings=lambda: settings,
        get_attractor_client=lambda: None,
    )
    await runtime.reload()

    launched_payloads: list[dict[str, object]] = []

    async def fake_launch_trigger_flow(definition, payload):
        launched_payloads.append({"trigger_id": definition.id, **payload})
        return f"run-{len(launched_payloads)}"

    runtime._launch_trigger_flow = fake_launch_trigger_flow  # type: ignore[method-assign]

    await runtime.emit_flow_event(
        {
            "run_id": "upstream-run-1",
            "flow_name": "upstream.dot",
            "project_path": str((tmp_path / "project").resolve()),
            "status": "completed",
        }
    )
    await runtime.emit_flow_event(
        {
            "run_id": "upstream-run-1",
            "flow_name": "upstream.dot",
            "project_path": str((tmp_path / "project").resolve()),
            "status": "completed",
        }
    )
    await asyncio.sleep(0)
    assert sum(1 for payload in launched_payloads if payload.get("trigger_id")) == 2

    poll_definition = next(definition for definition in runtime._definitions.values() if definition.source_type == "poll")
    await runtime._schedule_trigger_fire(poll_definition, {"poll_item": {"id": "item-1", "value": 1}})
    await runtime._schedule_trigger_fire(poll_definition, {"poll_item": {"id": "item-1", "value": 1}})
    await asyncio.sleep(0)

    assert [
        payload["poll_item"]
        for payload in launched_payloads
        if "poll_item" in payload
    ] == [
        {"id": "item-1", "value": 1},
        {"id": "item-1", "value": 1},
    ]
