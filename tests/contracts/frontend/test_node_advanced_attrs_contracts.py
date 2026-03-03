from __future__ import annotations

from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_test_passed
from tests.contracts.frontend._support.dot_probe import run_dot_utils_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def _generate_dot_with_advanced_node_attrs() -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  { id: 'start', data: { label: 'Start', shape: 'Mdiamond' } },
  {
    id: 'task',
    data: {
      label: 'Task',
      shape: 'box',
      prompt: 'Do work',
      max_retries: '2',
      goal_gate: true,
      retry_target: 'fix',
      fallback_retry_target: 'done',
      fidelity: 'summary:low',
      thread_id: 'thread-a',
      class: 'critical',
      timeout: '900s',
      llm_model: 'gpt-5',
      llm_provider: 'openai',
      reasoning_effort: 'high',
      auto_status: true,
      allow_partial: true
    }
  },
  {
    id: 'gate',
    data: {
      label: 'Gate',
      shape: 'hexagon',
      prompt: 'Choose path',
      'human.default_choice': 'fix'
    }
  },
  { id: 'fix', data: { label: 'Fix', shape: 'box', prompt: 'Fix issue' } },
  { id: 'done', data: { label: 'Done', shape: 'Msquare' } }
]
const edges = [
  { id: 'e1', source: 'start', target: 'task' },
  { id: 'e2', source: 'task', target: 'gate' },
  { id: 'e3', source: 'gate', target: 'fix', data: { label: 'Fix' } },
  { id: 'e4', source: 'gate', target: 'done', data: { label: 'Ship' } },
  { id: 'e5', source: 'fix', target: 'done' }
]
const dot = mod.generateDot('node_advanced_attrs_probe', nodes, edges, {})
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-node-advanced-",
        error_context="node advanced attrs probe",
    )


def test_node_advanced_attr_controls_present_item_6_2_02() -> None:
    assert_frontend_behavior_test_passed(
        "renders advanced node controls for codergen and wait.human in sidebar inspector"
    )


def test_node_advanced_attrs_round_trip_through_preview_item_6_2_02() -> None:
    flow = _generate_dot_with_advanced_node_attrs()
    payload = preview_pipeline(flow)
    nodes = payload["graph"]["nodes"]

    task_node = next((node for node in nodes if node["id"] == "task"), None)
    gate_node = next((node for node in nodes if node["id"] == "gate"), None)

    assert task_node is not None
    assert gate_node is not None

    assert task_node["max_retries"] == 2
    assert task_node["goal_gate"] is True
    assert task_node["retry_target"] == "fix"
    assert task_node["fallback_retry_target"] == "done"
    assert task_node["fidelity"] == "summary:low"
    assert task_node["thread_id"] == "thread-a"
    assert task_node["class"] == "critical"
    assert task_node["timeout"] == "900s"
    assert task_node["llm_model"] == "gpt-5"
    assert task_node["llm_provider"] == "openai"
    assert task_node["reasoning_effort"] == "high"
    assert task_node["auto_status"] is True
    assert task_node["allow_partial"] is True
    assert gate_node["human.default_choice"] == "fix"
