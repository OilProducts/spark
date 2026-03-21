from __future__ import annotations

import json

from tests.contracts.frontend._support.dot_probe import run_dot_utils_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def _generate_dot_with_graph_attrs(graph_attrs: dict[str, object]) -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const graphAttrs = JSON.parse(process.env.GRAPH_ATTRS_JSON)
const dot = mod.generateDot('round_trip_probe', [], [], graphAttrs)
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-round-trip-",
        error_context="graph attr round-trip probe",
        env_extra={"GRAPH_ATTRS_JSON": json.dumps(graph_attrs)},
    )


def test_graph_attr_edit_round_trip_serializes_and_rehydrates_item_6_1_04() -> None:
    graph_attrs_input: dict[str, object] = {
        "sparkspawn.title": "Execution Planning",
        "sparkspawn.description": "Turn approved spec edits into execution plans.",
        "sparkspawn.launch_inputs": '[{"key":"context.request.summary","label":"Request Summary","type":"string","description":"Brief request summary.","required":true}]',
        "goal": "Ship release",
        "label": "Release Graph",
        "model_stylesheet": ".fast { llm_model: fast-model; }",
        "default_max_retries": "3",
        "retry_target": "retry_stage",
        "fallback_retry_target": "fallback_stage",
        "default_fidelity": "summary:medium",
        "stack.child_dotfile": "child.dot",
        "stack.child_workdir": "/tmp/child",
        "tool.hooks.pre": "echo pre-hook",
        "tool.hooks.post": "echo post-hook",
    }
    flow = _generate_dot_with_graph_attrs(graph_attrs_input)
    payload = preview_pipeline(flow)
    graph_attrs = payload["graph"]["graph_attrs"]

    assert graph_attrs["sparkspawn.title"] == "Execution Planning"
    assert graph_attrs["sparkspawn.description"] == "Turn approved spec edits into execution plans."
    assert graph_attrs["sparkspawn.launch_inputs"] == '[{"key":"context.request.summary","label":"Request Summary","type":"string","description":"Brief request summary.","required":true}]'
    assert graph_attrs["goal"] == "Ship release"
    assert graph_attrs["label"] == "Release Graph"
    assert graph_attrs["model_stylesheet"] == ".fast { llm_model: fast-model; }"
    assert graph_attrs["default_max_retries"] == 3
    assert graph_attrs["retry_target"] == "retry_stage"
    assert graph_attrs["fallback_retry_target"] == "fallback_stage"
    assert graph_attrs["default_fidelity"] == "summary:medium"
    assert graph_attrs["stack.child_dotfile"] == "child.dot"
    assert graph_attrs["stack.child_workdir"] == "/tmp/child"
    assert graph_attrs["tool.hooks.pre"] == "echo pre-hook"
    assert graph_attrs["tool.hooks.post"] == "echo post-hook"
