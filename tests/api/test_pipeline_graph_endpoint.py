from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server

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


def test_get_pipeline_graph_returns_svg_for_known_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    monkeypatch.setattr(server, "RUNS_ROOT", runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    svg_path = runs_root / run_id / "artifacts" / "graphviz" / "pipeline.svg"
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
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    response = api_client.get("/pipelines/missing-run/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_graph_returns_404_when_svg_is_unavailable(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])

    response = api_client.get(f"/pipelines/{run_id}/graph")

    assert response.status_code == 404
    assert response.json()["detail"] == "Graph visualization unavailable"
