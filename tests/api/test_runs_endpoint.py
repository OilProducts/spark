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

