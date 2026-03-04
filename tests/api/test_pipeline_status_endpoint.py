from __future__ import annotations

from pathlib import Path
import time

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


def _wait_for_pipeline_terminal_status(api_client: TestClient, pipeline_id: str) -> str:
    for _ in range(400):
        response = api_client.get(f"/pipelines/{pipeline_id}")
        assert response.status_code == 200
        status = str(response.json()["status"])
        if status != "running":
            return status
        time.sleep(0.01)
    raise AssertionError("timed out waiting for pipeline completion")


def _write_checkpoint(run_root: Path, current_node: str, completed_nodes: list[str]) -> None:
    save_checkpoint(
        run_root / "state.json",
        Checkpoint(
            current_node=current_node,
            completed_nodes=completed_nodes,
            context={},
            retry_counts={},
        ),
    )


def test_get_pipeline_returns_progress_for_active_run(
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
    _write_checkpoint(run_root, current_node="plan", completed_nodes=["start"])

    response = api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["status"] == "running"
    assert payload["completed_nodes"] == ["start"]
    assert payload["progress"] == {
        "current_node": "plan",
        "completed_count": 1,
    }


def test_get_pipeline_uses_checkpoint_progress_for_persisted_run(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    final_status = _wait_for_pipeline_terminal_status(api_client, run_id)
    assert final_status == "success"

    run_root = runs_root / run_id
    _write_checkpoint(run_root, current_node="done", completed_nodes=["start", "plan"])

    response = api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["status"] == "success"
    assert payload["completed_nodes"] == ["start", "plan"]
    assert payload["progress"] == {
        "current_node": "done",
        "completed_count": 2,
    }
