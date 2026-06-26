from __future__ import annotations

import json
import os
from pathlib import Path
import time
from typing import Mapping

import httpx
import pytest

from tests.compat import harness
from tests.compat.conftest import CompatServer


ITEM_ID = "M0-I03-HTTP-SSE-FIXTURES"
REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")
RUN_LIVE_DOT = """digraph G {
  graph [goal="Capture run live-event compatibility"]
  start [shape=Mdiamond]
  done [shape=Msquare]
  start -> done
}
"""


def test_sse_conversation_and_error_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    scenarios = [
        (
            "sse/live-conversation-snapshot",
            {
                "conversation_id": compat_server.conversation_id,
                "conversation_project_path": str(compat_server.project_dir),
            },
            1,
            5.0,
        ),
        (
            "sse/live-conversation-replay",
            {
                "conversation_id": compat_server.conversation_id,
                "conversation_project_path": str(compat_server.project_dir),
                "conversation_revision": 0,
            },
            1,
            5.0,
        ),
        (
            "sse/live-conversation-resync-unavailable",
            {
                "conversation_id": "missing-conversation",
                "conversation_project_path": str(compat_server.project_dir),
            },
            1,
            5.0,
        ),
        ("sse/live-run-unknown", {"run_id": "missing-run"}, 0, 5.0),
        ("sse/live-invalid-cursor", {"conversation_revision": -1}, 0, 5.0),
    ]
    for fixture_id, params, frame_limit, read_timeout in scenarios:
        manifest = compat_server.sse_manifest(
            fixture_id=fixture_id,
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            params=params,
            frame_limit=frame_limit,
            read_timeout=read_timeout,
        )
        _assert_sse_fixture(manifest, compat_fixture_root / f"{fixture_id}.json", pytestconfig)


def test_sse_trigger_and_keepalive_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    _assert_sse_fixture(
        compat_server.sse_manifest(
            fixture_id="sse/live-trigger-snapshot",
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            params={
                "include_triggers": True,
                "triggers_project_path": str(compat_server.project_dir),
            },
            frame_limit=1,
        ),
        compat_fixture_root / "sse/live-trigger-snapshot.json",
        pytestconfig,
    )

    created_trigger_ids: list[str] = []

    def create_and_delete_trigger() -> None:
        payload = json.loads(compat_server.trigger_payload_path.read_text(encoding="utf-8"))
        with httpx.Client(base_url=compat_server.base_url, timeout=10.0) as client:
            create_response = client.post("/workspace/api/triggers", json=payload)
            assert create_response.status_code == 200, create_response.text
            trigger_id = str(create_response.json()["id"])
            created_trigger_ids.append(trigger_id)
            delete_response = client.delete(f"/workspace/api/triggers/{trigger_id}")
            assert delete_response.status_code == 200, delete_response.text

    _assert_sse_fixture(
        compat_server.sse_manifest(
            fixture_id="sse/live-trigger-upsert-delete",
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            params={
                "include_triggers": True,
                "triggers_project_path": str(compat_server.project_dir),
            },
            frame_limit=3,
            after_frame={1: create_and_delete_trigger},
            read_timeout=10.0,
        ),
        compat_fixture_root / "sse/live-trigger-upsert-delete.json",
        pytestconfig,
    )
    assert created_trigger_ids

    _assert_sse_fixture(
        compat_server.sse_manifest(
            fixture_id="sse/live-keepalive-empty",
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            frame_limit=1,
            read_timeout=20.0,
        ),
        compat_fixture_root / "sse/live-keepalive-empty.json",
        pytestconfig,
    )


def test_sse_run_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    run_id = "run-000000000001"
    _start_deterministic_run(compat_server, run_id)

    _assert_sse_fixture(
        compat_server.sse_manifest(
            fixture_id="sse/live-run-replay",
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            params={"run_id": run_id, "run_sequence": 0},
            frame_limit=2,
            read_timeout=10.0,
        ),
        compat_fixture_root / "sse/live-run-replay.json",
        pytestconfig,
    )
    _assert_sse_fixture(
        compat_server.sse_manifest(
            fixture_id="sse/live-runs-overview",
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            params={
                "include_runs_overview": True,
                "runs_project_path": str(compat_server.project_dir),
            },
            frame_limit=1,
            read_timeout=10.0,
        ),
        compat_fixture_root / "sse/live-runs-overview.json",
        pytestconfig,
    )


def _assert_sse_fixture(
    manifest: Mapping[str, object],
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


def _update_goldens(pytestconfig: pytest.Config) -> bool:
    return bool(pytestconfig.getoption("--compat-update-goldens")) or os.environ.get(
        "SPARK_COMPAT_UPDATE_GOLDENS"
    ) == "1"


def _start_deterministic_run(compat_server: CompatServer, run_id: str) -> None:
    with httpx.Client(base_url=compat_server.base_url, timeout=10.0) as client:
        response = client.post(
            "/attractor/pipelines",
            json={
                "run_id": run_id,
                "flow_content": RUN_LIVE_DOT,
                "working_directory": str(compat_server.project_dir),
                "model": "compat-model",
            },
        )
        assert response.status_code == 200, response.text
        payload = response.json()
        assert payload["status"] == "started"
        assert payload["pipeline_id"] == run_id
        _wait_for_run_completion(client, run_id)
        _wait_for_run_journal(client, run_id, min_entries=2)


def _wait_for_run_completion(client: httpx.Client, run_id: str) -> None:
    deadline = time.monotonic() + 20
    last_payload: object = None
    while time.monotonic() < deadline:
        response = client.get(f"/attractor/pipelines/{run_id}")
        if response.status_code == 200:
            last_payload = response.json()
            status = str(last_payload.get("status") or "")
            if status in {"completed", "failed", "validation_error", "canceled"}:
                return
        else:
            last_payload = response.text
        time.sleep(0.2)
    raise AssertionError(f"run {run_id} did not reach terminal status: {last_payload!r}")


def _wait_for_run_journal(client: httpx.Client, run_id: str, *, min_entries: int) -> None:
    deadline = time.monotonic() + 10
    last_payload: object = None
    while time.monotonic() < deadline:
        response = client.get(f"/attractor/pipelines/{run_id}/journal")
        if response.status_code == 200:
            last_payload = response.json()
            entries = last_payload.get("entries")
            if isinstance(entries, list) and len(entries) >= min_entries:
                return
        else:
            last_payload = response.text
        time.sleep(0.2)
    raise AssertionError(f"run {run_id} did not expose {min_entries} journal entries: {last_payload!r}")
