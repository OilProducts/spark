from __future__ import annotations

from pathlib import Path

import pytest

import attractor.api.server as server
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def test_get_pipeline_graph_returns_svg_for_known_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    svg_path = server._run_root(run_id) / "artifacts" / "graphviz" / "pipeline.svg"
    svg_path.parent.mkdir(parents=True, exist_ok=True)
    svg_path.write_text("<svg/>", encoding="utf-8")

    response = api_client.get(f"/pipelines/{run_id}/graph")

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("image/svg+xml")
    assert response.text == "<svg/>"


def test_get_pipeline_graph_returns_404_for_unknown_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = api_client.get("/pipelines/missing-run/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_graph_returns_404_when_svg_is_unavailable(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    response = api_client.get(f"/pipelines/{run_id}/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Graph visualization unavailable"
