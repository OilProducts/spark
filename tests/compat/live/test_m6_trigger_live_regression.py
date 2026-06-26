from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, Mapping

import httpx
import pytest

from tests.compat import harness
from tests.compat.conftest import CompatServer


ITEM_ID = "M6-I04-TRIGGER-REGRESSION-AND-LIVE-GATE"
REQUIREMENTS = ("RR-WKF-004", "RR-WKF-005")
DECISIONS = ("CD-RR-007", "CD-RR-010", "CD-RR-013")


def test_live_trigger_stream_records_real_webhook_dispatch_upsert(
    rust_compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    created_trigger: dict[str, Any] = {}

    def create_trigger() -> None:
        created_trigger.update(_create_webhook_trigger(rust_compat_server))

    def dispatch_webhook() -> None:
        assert created_trigger
        _dispatch_webhook(rust_compat_server, created_trigger, {"payload": "m6-live-trigger"})

    manifest = rust_compat_server.sse_manifest(
        fixture_id="sse/live-trigger-dispatch-upsert",
        item_id=ITEM_ID,
        requirements=REQUIREMENTS,
        decisions=DECISIONS,
        params={
            "include_triggers": True,
            "triggers_project_path": str(rust_compat_server.project_dir),
        },
        frame_limit=3,
        after_frame={1: create_trigger, 2: dispatch_webhook},
        read_timeout=20.0,
    )

    _assert_sse_fixture(
        manifest,
        compat_fixture_root / "sse/live-trigger-dispatch-upsert.json",
        pytestconfig,
    )
    _assert_no_degraded_trigger_frames(manifest)
    frames = _event_frames(manifest)
    assert [frame["data"]["type"] for frame in frames] == [
        "trigger.snapshot",
        "trigger.upsert",
        "trigger.upsert",
    ]
    dispatch = frames[2]["data"]
    assert dispatch["resource"] == {"kind": "trigger", "id": created_trigger["id"]}
    state = dispatch["payload"]["trigger"]["state"]
    assert state["last_result"] == "success"
    assert state["recent_history"][0]["run_id"]


def test_live_runs_overview_stream_records_trigger_launched_run_upsert(
    rust_compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    def create_and_dispatch_webhook() -> None:
        trigger = _create_webhook_trigger(rust_compat_server)
        _dispatch_webhook(rust_compat_server, trigger, {"payload": "m6-live-run"})

    manifest = rust_compat_server.sse_manifest(
        fixture_id="sse/live-trigger-dispatch-run-upsert",
        item_id=ITEM_ID,
        requirements=REQUIREMENTS,
        decisions=DECISIONS,
        params={
            "include_runs_overview": True,
            "runs_project_path": str(rust_compat_server.project_dir),
        },
        frame_limit=2,
        after_frame={1: create_and_dispatch_webhook},
        read_timeout=20.0,
    )

    _assert_sse_fixture(
        manifest,
        compat_fixture_root / "sse/live-trigger-dispatch-run-upsert.json",
        pytestconfig,
    )
    frames = manifest["frames"]
    assert frames[0] == {"kind": "comment", "comments": ["keepalive"]}
    run_upsert = frames[1]["data"]
    assert run_upsert["type"] == "run.upsert"
    assert run_upsert["resource"] == {"kind": "runs_overview", "id": None}
    assert run_upsert["payload"]["run"]["flow_name"] == (
        "software-development/implement-change-request.dot"
    )
    assert run_upsert["payload"]["run"]["run_id"]


def _create_webhook_trigger(compat_server: CompatServer) -> dict[str, Any]:
    payload = json.loads(compat_server.trigger_payload_path.read_text(encoding="utf-8"))
    with httpx.Client(base_url=compat_server.base_url, timeout=10.0) as client:
        response = client.post("/workspace/api/triggers", json=payload)
    assert response.status_code == 200, response.text
    trigger = response.json()
    assert isinstance(trigger, dict)
    return trigger


def _dispatch_webhook(
    compat_server: CompatServer,
    trigger: Mapping[str, Any],
    payload: Mapping[str, Any],
) -> None:
    with httpx.Client(base_url=compat_server.base_url, timeout=20.0) as client:
        response = client.post(
            "/workspace/api/webhooks",
            headers={
                "X-Spark-Webhook-Key": str(trigger["source"]["webhook_key"]),
                "X-Spark-Webhook-Secret": str(trigger["webhook_secret"]),
                "X-Spark-Webhook-Request-Id": "m6-live-request",
            },
            json=dict(payload),
        )
    assert response.status_code == 200, response.text
    assert response.json() == {"ok": True, "trigger_id": trigger["id"]}


def _assert_sse_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    pytestconfig: pytest.Config,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
    if _update_goldens(pytestconfig):
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_sse_manifest_matches_golden(manifest, expected)


def _event_frames(manifest: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    return [
        frame
        for frame in manifest.get("frames", [])
        if isinstance(frame, Mapping) and frame.get("kind") == "event"
    ]


def _assert_no_degraded_trigger_frames(manifest: Mapping[str, Any]) -> None:
    for frame in _event_frames(manifest):
        data = frame.get("data")
        assert isinstance(data, Mapping)
        assert data.get("type") in {"trigger.snapshot", "trigger.upsert", "trigger.delete"}
        payload = data.get("payload")
        if isinstance(payload, Mapping):
            assert payload.get("type") not in {"trigger_degraded", "no_trigger"}


def _update_goldens(pytestconfig: pytest.Config) -> bool:
    return bool(pytestconfig.getoption("--compat-update-goldens")) or os.environ.get(
        "SPARK_COMPAT_UPDATE_GOLDENS"
    ) == "1"
