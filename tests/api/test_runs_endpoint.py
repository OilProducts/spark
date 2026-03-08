from __future__ import annotations

import json
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server


def test_list_runs_includes_project_and_git_metadata_fields(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-project-metadata"
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

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

    response = api_client.get("/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["project_path"] == str(tmp_path / "project")
    assert run_payload["git_branch"] == "main"
    assert run_payload["git_commit"] == "abc123"


def test_list_runs_includes_spec_and_plan_artifact_links_when_available_item_9_6_03(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-artifact-links"
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

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
            spec_id="spec-project-1700000000",
            plan_id="plan-project-1700000000",
        )
    )

    response = api_client.get("/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["spec_id"] == "spec-project-1700000000"
    assert run_payload["plan_id"] == "plan-project-1700000000"


def test_list_runs_filters_durable_history_by_project_item_9_6_01(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

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

    filtered_response = api_client.get(
        "/runs",
        params={"project_path": str(tmp_path / "project-alpha")},
    )
    assert filtered_response.status_code == 200
    filtered_payload = filtered_response.json()
    filtered_run_ids = {run["run_id"] for run in filtered_payload["runs"]}

    assert filtered_run_ids == {"run-in-project-root", "run-in-project-child"}


def test_list_runs_backfills_missing_timestamps_from_run_log_item_9_6_04(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

    run_id = "run-with-partial-timestamps"
    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project"),
            model="test-model",
            started_at="",
            ended_at=None,
            project_path=str(tmp_path / "project"),
        )
    )
    run_root = server._run_root(run_id)
    (run_root / "run.log").write_text(
        "\n".join(
            [
                "[2026-01-01 00:10:00 UTC] Starting run",
                "[2026-01-01 00:10:30 UTC] Pipeline success",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    response = api_client.get("/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["started_at"] == "2026-01-01T00:10:00Z"
    assert run_payload["ended_at"] == "2026-01-01T00:10:30Z"


def test_list_runs_reconstructs_timestamp_ordering_from_run_logs_item_9_6_04(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

    older_id = "run-older"
    newer_id = "run-newer"

    for run_id, start_ts, end_ts in [
        (older_id, "2026-01-01 00:00:00", "2026-01-01 00:00:30"),
        (newer_id, "2026-01-01 00:01:00", "2026-01-01 00:01:30"),
    ]:
        server._write_run_meta(
            server.RunRecord(
                run_id=run_id,
                flow_name="Flow",
                status="success",
                result="success",
                working_directory=str(tmp_path / "project"),
                model="test-model",
                started_at="",
                ended_at=None,
                project_path=str(tmp_path / "project"),
            )
        )
        run_root = server._run_root(run_id)
        (run_root / "run.log").write_text(
            "\n".join(
                [
                    f"[{start_ts} UTC] Starting run",
                    f"[{end_ts} UTC] Pipeline success",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

    response = api_client.get("/runs")
    assert response.status_code == 200
    payload = response.json()
    run_ids = [run["run_id"] for run in payload["runs"]]

    assert run_ids == [newer_id, older_id]
