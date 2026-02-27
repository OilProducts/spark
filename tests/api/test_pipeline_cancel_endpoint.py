from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from fastapi import HTTPException

import attractor.api.server as server


def test_cancel_pipeline_returns_404_for_unknown_pipeline(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    with pytest.raises(HTTPException) as exc_info:
        asyncio.run(server.cancel_pipeline("missing-run"))

    assert exc_info.value.status_code == 404
    assert exc_info.value.detail == "Unknown pipeline"


def test_cancel_pipeline_requests_cancel_for_active_run(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    run_id = "run-active"
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)

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

    captured_events: list[dict] = []

    async def _capture_event(_run_id: str, message: dict) -> None:
        captured_events.append(dict(message))

    monkeypatch.setattr(server, "_publish_run_event", _capture_event)

    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS[run_id] = server.ActiveRun(
            run_id=run_id,
            flow_name="Flow",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            status="running",
        )

    try:
        payload = asyncio.run(server.cancel_pipeline(run_id))
        active = server._get_active_run(run_id)
        assert active is not None
        assert active.control.poll() == "abort"
        assert active.status == "cancel_requested"
    finally:
        server._pop_active_run(run_id)

    record = server._read_run_meta(server._run_meta_path(run_id))
    assert record is not None
    assert record.status == "cancel_requested"
    assert record.result == "cancel_requested"
    assert record.last_error == "cancel_requested_by_user"

    assert payload == {"status": "cancel_requested", "pipeline_id": run_id}
    assert server.RUNTIME.status == "cancel_requested"
    assert server.RUNTIME.last_error == "cancel_requested_by_user"
    assert captured_events == [
        {"type": "runtime", "status": "cancel_requested"},
        {"type": "log", "msg": "[System] Cancel requested. Stopping after current node."},
    ]


def test_cancel_pipeline_ignores_non_running_known_pipeline(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    run_id = "run-finished"
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="success",
            result="success",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
        )
    )

    payload = asyncio.run(server.cancel_pipeline(run_id))

    assert payload == {"status": "ignored", "pipeline_id": run_id}
