from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def _seed_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> tuple[str, Path]:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = str(_start_pipeline(attractor_api_client, tmp_path / "work")["pipeline_id"])
    return run_id, server._run_root(run_id)


def test_list_pipeline_artifacts_returns_run_outputs_for_known_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id, run_root = _seed_run(attractor_api_client, monkeypatch, tmp_path)

    (run_root / "manifest.json").write_text("{}", encoding="utf-8")
    (run_root / "checkpoint.json").write_text("{}", encoding="utf-8")
    (run_root / "run.log").write_text("[2026-04-13 16:00:00 UTC] artifact-visible log\n", encoding="utf-8")

    stage_dir = run_root / "plan"
    stage_dir.mkdir(parents=True, exist_ok=True)
    (stage_dir / "prompt.md").write_text("# prompt", encoding="utf-8")
    (stage_dir / "response.md").write_text("# response", encoding="utf-8")
    (stage_dir / "status.json").write_text('{"outcome":"success"}', encoding="utf-8")
    rpc_dir = run_root / "logs" / "plan"
    rpc_dir.mkdir(parents=True, exist_ok=True)
    (rpc_dir / "raw-rpc.jsonl").write_text('{"direction":"incoming","line":"{}"}\n', encoding="utf-8")

    artifact_file = run_root / "artifacts" / "logs" / "output.txt"
    artifact_file.parent.mkdir(parents=True, exist_ok=True)
    artifact_file.write_text("done", encoding="utf-8")

    # Internal checkpoint state should not be exposed by the artifact browser list.
    (run_root / "state.json").write_text("{}", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/artifacts")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    paths = [item["path"] for item in payload["artifacts"]]
    for expected in (
        "artifacts/logs/output.txt",
        "checkpoint.json",
        "manifest.json",
        "logs/plan/raw-rpc.jsonl",
        "plan/prompt.md",
        "plan/response.md",
        "plan/status.json",
        "run.log",
    ):
        assert expected in paths
    assert "state.json" not in paths


def test_get_pipeline_artifact_file_returns_file_for_known_artifact(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id, run_root = _seed_run(attractor_api_client, monkeypatch, tmp_path)

    artifact_path = run_root / "plan" / "prompt.md"
    artifact_path.parent.mkdir(parents=True, exist_ok=True)
    artifact_path.write_text("# prompt", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/artifacts/plan/prompt.md")

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/markdown")
    assert response.text == "# prompt"


def test_get_pipeline_artifact_file_rejects_parent_traversal(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id, _ = _seed_run(attractor_api_client, monkeypatch, tmp_path)

    response = attractor_api_client.get(f"/pipelines/{run_id}/artifacts/%2E%2E/run.json")

    assert response.status_code == 400
    assert response.json()["detail"] == "Invalid artifact path"


def test_get_pipeline_artifact_file_returns_404_when_missing(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id, _ = _seed_run(attractor_api_client, monkeypatch, tmp_path)

    response = attractor_api_client.get(f"/pipelines/{run_id}/artifacts/plan/missing.md")

    assert response.status_code == 404
    assert response.json()["detail"] == "Artifact not found"
