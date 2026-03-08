from __future__ import annotations

from pathlib import Path

import pytest

import attractor.api.server as server
from attractor.engine import Checkpoint, save_checkpoint
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_terminal_status as _wait_for_pipeline_terminal_status,
)


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
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    run_root = server._run_root(run_id)
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
    server.configure_runtime_paths(runs_dir=runs_root)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    final_status = _wait_for_pipeline_terminal_status(api_client, run_id)
    assert final_status == "success"

    run_root = server._run_root(run_id)
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
