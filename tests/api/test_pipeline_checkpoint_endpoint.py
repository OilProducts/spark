from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine import Checkpoint, save_checkpoint

FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""


def _close_task_immediately(coro):
    coro.close()

    class _DummyTask:
        pass

    return _DummyTask()


def _start_pipeline(api_client: TestClient, working_directory: Path) -> dict:
    response = api_client.post(
        "/pipelines",
        json={
            "flow_content": FLOW,
            "working_directory": str(working_directory),
            "backend": "codex",
        },
    )
    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    return payload


def test_get_pipeline_checkpoint_returns_404_for_unknown_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    response = api_client.get("/pipelines/missing-run/checkpoint")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_checkpoint_returns_current_state_for_known_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    run_root = runs_root / run_id

    checkpoint = Checkpoint(
        timestamp="2026-01-01T00:00:00Z",
        current_node="implement",
        completed_nodes=["start", "plan"],
        context={"graph.goal": "Ship feature", "outcome": "success"},
        retry_counts={"implement": 1},
        logs=["started", "implemented"],
    )
    save_checkpoint(run_root / "state.json", checkpoint)

    response = api_client.get(f"/pipelines/{run_id}/checkpoint")

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "pipeline_id": run_id,
        "checkpoint": checkpoint.to_dict(),
    }
