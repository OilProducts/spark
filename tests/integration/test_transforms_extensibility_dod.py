from __future__ import annotations

import asyncio

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


def test_transform_pipeline_requires_apply_method(monkeypatch: pytest.MonkeyPatch) -> None:
    class _InvalidTransform:
        def transform(self, graph):
            return graph

    server.clear_registered_transforms()
    try:
        server.register_transform(_InvalidTransform())
        with pytest.raises(TypeError, match="Transform must implement apply\\(graph\\)"):
            asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=FLOW_WITH_GOAL)))
    finally:
        server.clear_registered_transforms()
