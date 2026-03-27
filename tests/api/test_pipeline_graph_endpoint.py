from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.graphviz_export import GraphvizArtifactExport
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def test_get_pipeline_graph_returns_svg_for_known_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    svg_path = server._run_root(run_id) / "artifacts" / "graphviz" / "pipeline.svg"
    svg_path.parent.mkdir(parents=True, exist_ok=True)
    svg_path.write_text("<svg/>", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph")

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("image/svg+xml")
    assert response.text == "<svg/>"


def test_get_pipeline_graph_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = attractor_api_client.get("/pipelines/missing-run/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_graph_returns_404_when_svg_is_unavailable(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    monkeypatch.setattr(
        server,
        "export_graphviz_artifact",
        lambda dot_source, run_root: GraphvizArtifactExport(
            source_path=Path(run_root) / "artifacts" / "graphviz" / "pipeline-source.dot",
            dot_path=Path(run_root) / "artifacts" / "graphviz" / "pipeline.dot",
            rendered_path=None,
            error="Graphviz render forced unavailable for test",
        ),
    )
    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Graph visualization unavailable"


def test_get_pipeline_graph_preview_returns_prepared_preview_for_known_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    source_path = server._run_root(run_id) / "artifacts" / "graphviz" / "pipeline-source.dot"
    source_path.parent.mkdir(parents=True, exist_ok=True)
    source_path.write_text("digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph-preview")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "ok"
    assert "graph" in payload
    assert isinstance(payload["graph"]["nodes"], list)
    assert isinstance(payload["graph"]["edges"], list)


def test_get_pipeline_graph_preview_falls_back_to_pipeline_dot_when_source_snapshot_is_unavailable(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    graph_dir = server._run_root(run_id) / "artifacts" / "graphviz"
    source_path = graph_dir / "pipeline-source.dot"
    source_path.unlink(missing_ok=True)
    dot_path = graph_dir / "pipeline.dot"
    dot_path.write_text("digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph-preview")

    assert response.status_code == 200
    assert response.json()["status"] == "ok"


def test_get_pipeline_graph_preview_returns_parse_error_payload_for_invalid_snapshot(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    source_path = server._run_root(run_id) / "artifacts" / "graphviz" / "pipeline-source.dot"
    source_path.write_text("digraph G { start ->", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph-preview")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "parse_error"
    assert payload["errors"]


def test_get_pipeline_graph_preview_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = attractor_api_client.get("/pipelines/missing-run/graph-preview")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_graph_preview_returns_404_when_snapshot_is_unavailable(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    graph_dir = server._run_root(run_id) / "artifacts" / "graphviz"
    (graph_dir / "pipeline-source.dot").unlink(missing_ok=True)
    (graph_dir / "pipeline.dot").unlink(missing_ok=True)

    response = attractor_api_client.get(f"/pipelines/{run_id}/graph-preview")

    assert response.status_code == 404
    assert response.json()["detail"] == "Run graph preview unavailable"
