from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from tests.api._support import start_pipeline, wait_for_pipeline_completion


class ResultBackend:
    def __init__(self, responses: dict[str, str]):
        self.responses = responses
        self.calls: list[tuple[str, str]] = []

    def run(self, node_id: str, prompt: str, context: Any, **kwargs: Any) -> str:
        del context, kwargs
        self.calls.append((node_id, prompt))
        if node_id == "result_summary":
            return f"summary: {prompt.split('Source result:', 1)[-1].strip()}"
        return self.responses.get(node_id, f"response: {node_id}")


def _start_completed_result_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    *,
    flow_content: str,
    backend: ResultBackend,
) -> tuple[str, Path]:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    started = start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content=flow_content,
        backend="provider-router",
    )
    run_id = str(started["pipeline_id"])
    completed = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert completed["status"] == "completed"
    return run_id, server._run_root(run_id)


def test_pipeline_result_uses_explicit_result_node(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = ResultBackend({"first": "first raw", "second": "second raw"})
    flow = """
digraph G {
  graph [spark.result_node="first"];
  start [shape=Mdiamond];
  first [shape=box];
  second [shape=box];
  done [shape=Msquare];
  start -> first -> second -> done;
}
"""
    run_id, run_root = _start_completed_result_run(
        attractor_api_client,
        monkeypatch,
        tmp_path,
        flow_content=flow,
        backend=backend,
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}/result")

    assert response.status_code == 200
    payload = response.json()
    assert payload["state"] == "ready"
    assert payload["source_node_id"] == "first"
    assert payload["source_artifact_path"] == "logs/first/response.md"
    assert payload["display_mode"] == "raw"
    assert payload["body_markdown"].strip() == "first raw"
    assert (run_root / "result" / "result.md").read_text(encoding="utf-8").strip() == "first raw"


def test_pipeline_result_falls_back_to_last_successful_work_node_before_exit(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = ResultBackend({"first": "first raw", "second": "second raw"})
    flow = """
digraph G {
  start [shape=Mdiamond];
  first [shape=box];
  second [shape=box];
  done [shape=Msquare];
  start -> first -> second -> done;
}
"""
    run_id, _ = _start_completed_result_run(
        attractor_api_client,
        monkeypatch,
        tmp_path,
        flow_content=flow,
        backend=backend,
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}/result")

    assert response.status_code == 200
    payload = response.json()
    assert payload["state"] == "ready"
    assert payload["source_node_id"] == "second"
    assert payload["display_mode"] == "raw"
    assert payload["body_markdown"].strip() == "second raw"


def test_pipeline_result_summary_enabled_writes_and_returns_summary(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = ResultBackend({"answer": "raw answer"})
    flow = """
digraph G {
  graph [spark.result_summary_enabled="true", spark.result_summary_prompt="Summarize this"];
  start [shape=Mdiamond];
  answer [shape=box];
  done [shape=Msquare];
  start -> answer -> done;
}
"""
    run_id, run_root = _start_completed_result_run(
        attractor_api_client,
        monkeypatch,
        tmp_path,
        flow_content=flow,
        backend=backend,
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}/result")

    assert response.status_code == 200
    payload = response.json()
    assert payload["state"] == "ready"
    assert payload["source_node_id"] == "answer"
    assert payload["display_mode"] == "summary"
    assert payload["body_markdown"].strip() == "summary: raw answer"
    assert payload["summary_enabled"] is True
    assert payload["summary_prompt"] == "Summarize this"
    assert (run_root / "result" / "result.md").read_text(encoding="utf-8").strip() == "summary: raw answer"
    assert backend.calls[-1][0] == "result_summary"
    assert "Summarize this" in backend.calls[-1][1]


def test_pipeline_result_unavailable_when_no_source_exists_without_changing_run_status(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = ResultBackend({})
    flow = """
digraph G {
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
"""
    run_id, _ = _start_completed_result_run(
        attractor_api_client,
        monkeypatch,
        tmp_path,
        flow_content=flow,
        backend=backend,
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}/result")
    status_response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    assert response.json()["state"] == "unavailable"
    assert status_response.status_code == 200
    assert status_response.json()["status"] == "completed"


def test_pipeline_result_unavailable_for_invalid_explicit_result_node(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = ResultBackend({"answer": "raw answer"})
    flow = """
digraph G {
  graph [spark.result_node="missing"];
  start [shape=Mdiamond];
  answer [shape=box];
  done [shape=Msquare];
  start -> answer -> done;
}
"""
    run_id, _ = _start_completed_result_run(
        attractor_api_client,
        monkeypatch,
        tmp_path,
        flow_content=flow,
        backend=backend,
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}/result")

    assert response.status_code == 200
    assert response.json()["state"] == "unavailable"
