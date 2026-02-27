from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import attractor.api.server as server


FLOW_WITH_GOAL = """
digraph G {
    graph [goal="Ship docs"]
    start [shape=Mdiamond]
    task [shape=box, prompt="Build $goal"]
    done [shape=Msquare]
    start -> task -> done
}
"""


def _find_route(path: str, method: str):
    for route in server.app.routes:
        if getattr(route, "path", "") != path:
            continue
        methods = getattr(route, "methods", set())
        if method in methods:
            return route
    return None


def test_transform_interface_can_modify_graph_between_parse_and_validation(monkeypatch: pytest.MonkeyPatch) -> None:
    class _AppendPromptTransform:
        def apply(self, graph):
            prompt_attr = graph.nodes["task"].attrs["prompt"]
            prompt_attr.value = f"{prompt_attr.value} [custom]"
            return graph

    captured: dict[str, str] = {}

    def _validate(graph):
        captured["prompt"] = str(graph.nodes["task"].attrs["prompt"].value)
        return []

    monkeypatch.setattr(server, "validate_graph", _validate)
    server.clear_registered_transforms()
    try:
        server.register_transform(_AppendPromptTransform())
        payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=FLOW_WITH_GOAL)))
    finally:
        server.clear_registered_transforms()

    assert payload["status"] == "ok"
    # Built-in $goal expansion runs before validation, then custom transform appends.
    assert captured["prompt"] == "Build Ship docs [custom]"


def test_custom_transforms_run_in_registration_order(monkeypatch: pytest.MonkeyPatch) -> None:
    class _AppendFirstTransform:
        def apply(self, graph):
            prompt_attr = graph.nodes["task"].attrs["prompt"]
            prompt_attr.value = f"{prompt_attr.value} [first]"
            return graph

    class _AppendSecondTransform:
        def apply(self, graph):
            prompt_attr = graph.nodes["task"].attrs["prompt"]
            prompt_attr.value = f"{prompt_attr.value} [second]"
            return graph

    captured: dict[str, str] = {}

    def _validate(graph):
        captured["prompt"] = str(graph.nodes["task"].attrs["prompt"].value)
        return []

    monkeypatch.setattr(server, "validate_graph", _validate)
    server.clear_registered_transforms()
    try:
        server.register_transform(_AppendFirstTransform())
        server.register_transform(_AppendSecondTransform())
        payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=FLOW_WITH_GOAL)))
    finally:
        server.clear_registered_transforms()

    assert payload["status"] == "ok"
    assert captured["prompt"] == "Build Ship docs [first] [second]"


def test_transform_pipeline_accepts_transform_method_compatibility(monkeypatch: pytest.MonkeyPatch) -> None:
    class _LegacyTransform:
        def transform(self, graph):
            prompt_attr = graph.nodes["task"].attrs["prompt"]
            prompt_attr.value = f"{prompt_attr.value} [legacy]"
            return graph

    captured: dict[str, str] = {}

    def _validate(graph):
        captured["prompt"] = str(graph.nodes["task"].attrs["prompt"].value)
        return []

    monkeypatch.setattr(server, "validate_graph", _validate)
    server.clear_registered_transforms()
    try:
        server.register_transform(_LegacyTransform())
        payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=FLOW_WITH_GOAL)))
    finally:
        server.clear_registered_transforms()

    assert payload["status"] == "ok"
    assert captured["prompt"] == "Build Ship docs [legacy]"


def test_http_server_mode_registers_run_status_and_answer_routes() -> None:
    assert _find_route("/run", "POST") is not None
    assert _find_route("/status", "GET") is not None
    assert _find_route("/answer", "POST") is not None


def test_status_endpoint_returns_runtime_snapshot(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(server.RUNTIME, "status", "running")
    monkeypatch.setattr(server.RUNTIME, "last_error", "none")
    monkeypatch.setattr(server.RUNTIME, "last_working_directory", "/tmp/work")
    monkeypatch.setattr(server.RUNTIME, "last_model", "gpt-test")
    monkeypatch.setattr(server.RUNTIME, "last_completed_nodes", ["start", "task"])
    monkeypatch.setattr(server.RUNTIME, "last_flow_name", "Flow")
    monkeypatch.setattr(server.RUNTIME, "last_run_id", "run-xyz")

    payload = asyncio.run(server.get_status())

    assert payload == {
        "status": "running",
        "last_error": "none",
        "last_working_directory": "/tmp/work",
        "last_model": "gpt-test",
        "last_completed_nodes": ["start", "task"],
        "last_flow_name": "Flow",
        "last_run_id": "run-xyz",
    }


def test_run_endpoint_delegates_to_start_pipeline(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    captured: dict[str, server.PipelineStartRequest] = {}

    async def _fake_start(req: server.PipelineStartRequest) -> dict:
        captured["request"] = req
        return {"status": "started", "pipeline_id": "legacy-run"}

    monkeypatch.setattr(server, "_start_pipeline", _fake_start)

    payload = asyncio.run(
        server.run_pipeline(
            server.PipelineStartRequest(
                flow_content=FLOW_WITH_GOAL,
                working_directory=str(tmp_path / "work"),
            )
        )
    )

    assert payload == {"status": "started", "pipeline_id": "legacy-run"}
    assert "request" in captured
    assert captured["request"].flow_content.strip().startswith("digraph G")


def test_answer_endpoint_delegates_to_pipeline_question_answer(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, object] = {}

    async def _fake_submit(pipeline_id: str, question_id: str, req: server.HumanAnswerRequest) -> dict:
        captured["pipeline_id"] = pipeline_id
        captured["question_id"] = question_id
        captured["selected_value"] = req.selected_value
        return {"status": "accepted", "pipeline_id": pipeline_id, "question_id": question_id}

    monkeypatch.setattr(server, "submit_pipeline_answer", _fake_submit)

    payload = asyncio.run(
        server.answer_pipeline(
            server.LegacyHumanAnswerRequest(
                pipeline_id="run-123",
                question_id="q-1",
                selected_value="approve",
            )
        )
    )

    assert payload == {"status": "accepted", "pipeline_id": "run-123", "question_id": "q-1"}
    assert captured == {
        "pipeline_id": "run-123",
        "question_id": "q-1",
        "selected_value": "approve",
    }
