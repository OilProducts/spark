from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any, Mapping

import httpx

from tests.compat import harness
from tests.compat.conftest import CompatServer


ITEM_ID = "M6-I04-TRIGGER-REGRESSION-AND-LIVE-GATE"
REQUIREMENTS = ("RR-WKF-004",)
DECISIONS = ("CD-RR-007", "CD-RR-010", "CD-RR-013")


def test_webhook_dispatch_persists_runtime_state_and_run_provenance(
    rust_compat_server: CompatServer,
) -> None:
    harness.validate_manifest_coverage(
        {"requirements": REQUIREMENTS, "decisions": DECISIONS},
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
    create_payload = json.loads(rust_compat_server.trigger_payload_path.read_text(encoding="utf-8"))
    with httpx.Client(base_url=rust_compat_server.base_url, timeout=20.0) as client:
        create_response = client.post("/workspace/api/triggers", json=create_payload)
        assert create_response.status_code == 200, create_response.text
        trigger = create_response.json()
        trigger_id = str(trigger["id"])

        webhook_response = client.post(
            "/workspace/api/webhooks",
            headers={
                "X-Spark-Webhook-Key": str(trigger["source"]["webhook_key"]),
                "X-Spark-Webhook-Secret": str(trigger["webhook_secret"]),
                "X-Spark-Webhook-Request-Id": "m6-api-request",
            },
            json={"payload": "m6-api"},
        )
        assert webhook_response.status_code == 200, webhook_response.text
        assert webhook_response.json() == {"ok": True, "trigger_id": trigger_id}

        state = _wait_for_trigger_state(rust_compat_server.spark_home, trigger_id)
        assert state["last_result"] == "success"
        assert state["last_error"] is None
        history = state["recent_history"]
        assert history and history[0]["status"] == "success"
        run_id = str(history[0]["run_id"])

        detail_response = client.get(f"/attractor/pipelines/{run_id}")
        assert detail_response.status_code == 200, detail_response.text
        assert detail_response.json()["pipeline_id"] == run_id

        context_response = client.get(f"/attractor/pipelines/{run_id}/context")
        assert context_response.status_code == 200, context_response.text
        context = context_response.json()["context"]
        assert context["context.trigger_static"] == {"origin": "compat"}
        assert context["context.trigger_payload"] == {"payload": "m6-api"}
        assert context["context.spark_trigger"] == {
            "trigger_id": trigger_id,
            "trigger_name": "Compat webhook",
            "source_type": "webhook",
        }


def _wait_for_trigger_state(spark_home: Path, trigger_id: str) -> Mapping[str, Any]:
    state_path = spark_home / "workspace" / "trigger-state" / f"{trigger_id}.json"
    deadline = time.monotonic() + 10
    last_payload: object = None
    while time.monotonic() < deadline:
        if state_path.exists():
            last_payload = json.loads(state_path.read_text(encoding="utf-8"))
            if isinstance(last_payload, Mapping) and last_payload.get("last_result") == "success":
                return last_payload
        time.sleep(0.1)
    raise AssertionError(f"trigger state did not record dispatch success: {last_payload!r}")
