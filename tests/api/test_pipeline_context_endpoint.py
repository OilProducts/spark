from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from fastapi import HTTPException

import attractor.api.server as server
from attractor.engine import Checkpoint, save_checkpoint


def test_get_pipeline_context_returns_404_for_unknown_pipeline(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    with pytest.raises(HTTPException) as exc_info:
        asyncio.run(server.get_pipeline_context("missing-run"))

    assert exc_info.value.status_code == 404
    assert exc_info.value.detail == "Unknown pipeline"


def test_get_pipeline_context_returns_context_for_known_pipeline(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    run_id = "run-with-context"
    runs_root = tmp_path / "runs"
    run_root = runs_root / run_id
    run_root.mkdir(parents=True)
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)

    checkpoint = Checkpoint(
        timestamp="2026-01-01T00:00:00Z",
        current_node="implement",
        completed_nodes=["start", "plan"],
        context={
            "graph.goal": "Ship feature",
            "outcome": "success",
            "context.plan.ready": True,
        },
        retry_counts={"implement": 1},
        logs=["started", "implemented"],
    )
    save_checkpoint(run_root / "state.json", checkpoint)

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="running",
            result=None,
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
        )
    )

    payload = asyncio.run(server.get_pipeline_context(run_id))

    assert payload == {
        "pipeline_id": run_id,
        "context": checkpoint.context,
    }
