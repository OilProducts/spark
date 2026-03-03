from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import attractor.api.server as server


def test_list_runs_includes_project_and_git_metadata_fields(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    run_id = "run-with-project-metadata"
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)

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
            project_path=str(tmp_path / "project"),
            git_branch="main",
            git_commit="abc123",
            last_error="",
            token_usage=42,
        )
    )

    payload = asyncio.run(server.list_runs())

    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["project_path"] == str(tmp_path / "project")
    assert run_payload["git_branch"] == "main"
    assert run_payload["git_commit"] == "abc123"


def test_list_runs_filters_durable_history_by_project_item_9_6_01(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)

    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-root",
            flow_name="Flow A",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project-alpha"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project-alpha"),
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-child",
            flow_name="Flow B",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project-alpha" / "nested"),
            model="test-model",
            started_at="2026-01-01T00:02:00Z",
            ended_at="2026-01-01T00:03:00Z",
            project_path="",
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-other-project",
            flow_name="Flow C",
            status="failed",
            result="failed",
            working_directory=str(tmp_path / "project-beta"),
            model="test-model",
            started_at="2026-01-01T00:04:00Z",
            ended_at="2026-01-01T00:05:00Z",
            project_path=str(tmp_path / "project-beta"),
        )
    )

    filtered_payload = asyncio.run(server.list_runs(project_path=str(tmp_path / "project-alpha")))
    filtered_run_ids = {run["run_id"] for run in filtered_payload["runs"]}

    assert filtered_run_ids == {"run-in-project-root", "run-in-project-child"}
