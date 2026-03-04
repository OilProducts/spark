from __future__ import annotations

import json

from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_contract_passed
from tests.contracts.frontend._support.dot_probe import run_dot_utils_probe, run_graph_attr_validation_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def test_graph_settings_exposes_graph_scope_tool_hook_fields_item_6_6_01() -> None:
    assert_frontend_behavior_contract_passed("6.6.01")


def _generate_dot_with_node_tool_hook_overrides() -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  { id: 'start', data: { label: 'Start', shape: 'Mdiamond' } },
  {
    id: 'tool_node',
    data: {
      label: 'Tool',
      shape: 'parallelogram',
      type: 'tool',
      tool_command: 'echo run',
      'tool_hooks.pre': 'echo node pre',
      'tool_hooks.post': 'echo node post'
    }
  },
  { id: 'end', data: { label: 'End', shape: 'Msquare' } }
]
const edges = [
  { id: 'e1', source: 'start', target: 'tool_node' },
  { id: 'e2', source: 'tool_node', target: 'end' }
]
const graphAttrs = {
  'tool_hooks.pre': 'echo graph pre',
  'tool_hooks.post': 'echo graph post'
}
const dot = mod.generateDot('node_tool_hooks_probe', nodes, edges, graphAttrs)
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-node-tool-hooks-",
        error_context="node tool hooks probe",
    )


def test_node_tool_hook_override_controls_present_item_6_6_02() -> None:
    assert_frontend_behavior_contract_passed("6.6.02")


def test_tool_hook_warning_surfaces_present_item_6_6_03() -> None:
    assert_frontend_behavior_contract_passed("6.6.03")


def _probe_tool_hook_command_warning() -> dict[str, str | None]:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.GRAPH_ATTR_VALIDATION_JS_PATH).href)
console.log(JSON.stringify({
  valid: mod.getToolHookCommandWarning('echo hello'),
  embeddedApostrophe: mod.getToolHookCommandWarning(`echo "it's ok"`),
  newline: mod.getToolHookCommandWarning('echo hi\\necho there'),
  singleQuote: mod.getToolHookCommandWarning("echo 'unterminated"),
  doubleQuote: mod.getToolHookCommandWarning('echo "unterminated'),
}))
""".strip()

    output = run_graph_attr_validation_probe(
        probe_script,
        temp_prefix=".tmp-tool-hook-warning-",
        error_context="tool hook warning heuristic probe",
    )
    return json.loads(output)


def test_tool_hook_warning_heuristics_item_6_6_03() -> None:
    probe = _probe_tool_hook_command_warning()

    assert probe["valid"] is None
    assert probe["embeddedApostrophe"] is None
    assert probe["newline"] is not None and "single line" in probe["newline"].lower()
    assert probe["singleQuote"] is not None and "single quote" in probe["singleQuote"].lower()
    assert probe["doubleQuote"] is not None and "double quote" in probe["doubleQuote"].lower()


def test_node_tool_hook_overrides_round_trip_through_preview_item_6_6_02() -> None:
    flow = _generate_dot_with_node_tool_hook_overrides()
    payload = preview_pipeline(flow)
    nodes = payload["graph"]["nodes"]
    tool_node = next((node for node in nodes if node["id"] == "tool_node"), None)

    assert tool_node is not None
    assert tool_node["tool_command"] == "echo run"
    assert tool_node["tool_hooks.pre"] == "echo node pre"
    assert tool_node["tool_hooks.post"] == "echo node post"

    graph_attrs = payload["graph"]["graph_attrs"]
    assert graph_attrs["tool_hooks.pre"] == "echo graph pre"
    assert graph_attrs["tool_hooks.post"] == "echo graph post"


def _save_loaded_tool_hook_graph_via_generate_dot(flow_content: str) -> str:
    preview = preview_pipeline(flow_content)

    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const preview = JSON.parse(process.env.PREVIEW_JSON)

const nodes = preview.graph.nodes.map((n) => ({
  id: n.id,
  data: {
    label: n.label,
    shape: n.shape ?? 'box',
    prompt: n.prompt ?? '',
    tool_command: n.tool_command ?? '',
    'tool_hooks.pre': n['tool_hooks.pre'] ?? '',
    'tool_hooks.post': n['tool_hooks.post'] ?? '',
    type: n.type ?? ''
  }
}))

const edges = preview.graph.edges.map((e, i) => ({
  id: `e-${e.from}-${e.to}-${i}`,
  source: e.from,
  target: e.to
}))

const dot = mod.generateDot('tool_hooks_save_load_probe', nodes, edges, preview.graph.graph_attrs || {})
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-tool-hook-save-load-",
        error_context="tool-hook save/load probe",
        env_extra={"PREVIEW_JSON": json.dumps(preview)},
    )


def test_tool_hook_definitions_round_trip_through_save_load_item_6_6_04() -> None:
    flow = """
digraph tool_hook_save_load {
  graph [
    tool_hooks.pre="python ./hooks/pre.py --mode \\\"global\\\"",
    tool_hooks.post="./hooks/post.sh --emit report"
  ];
  start [label="Start", shape=Mdiamond];
  tool_node [
    label="Tool",
    shape=parallelogram,
    type=tool,
    tool_command="echo run",
    tool_hooks.pre="./hooks/node_pre.sh --flag",
    tool_hooks.post="python -c \\\"print('done')\\\""
  ];
  end [label="End", shape=Msquare];

  start -> tool_node;
  tool_node -> end;
}
""".strip()

    saved_dot = _save_loaded_tool_hook_graph_via_generate_dot(flow)
    payload = preview_pipeline(saved_dot)

    graph_attrs = payload["graph"]["graph_attrs"]
    assert graph_attrs["tool_hooks.pre"] == 'python ./hooks/pre.py --mode "global"'
    assert graph_attrs["tool_hooks.post"] == "./hooks/post.sh --emit report"

    nodes = payload["graph"]["nodes"]
    tool_node = next((node for node in nodes if node["id"] == "tool_node"), None)
    assert tool_node is not None
    assert tool_node["tool_hooks.pre"] == "./hooks/node_pre.sh --flag"
    assert tool_node["tool_hooks.post"] == 'python -c "print(\'done\')"'
