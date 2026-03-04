from __future__ import annotations

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_contract_passed
from tests.contracts.frontend._support.dot_probe import run_dot_utils_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def _generate_dot_with_edge_attrs() -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  { id: 'start', data: { label: 'Start', shape: 'Mdiamond' } },
  { id: 'route', data: { label: 'Route', shape: 'diamond', type: 'conditional', prompt: 'Route work' } },
  { id: 'done', data: { label: 'Done', shape: 'Msquare' } }
]
const edges = [
  {
    id: 'e1',
    source: 'start',
    target: 'route',
    data: {
      label: 'success',
      condition: 'outcome=success && context.tests_passed=true',
      weight: '7',
      fidelity: 'summary:low',
      thread_id: 'review-thread',
      loop_restart: true
    }
  },
  { id: 'e2', source: 'route', target: 'done' }
]
const dot = mod.generateDot('edge_attrs_probe', nodes, edges, {})
console.log(dot)
""".strip()
    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-edge-attrs-",
        error_context="edge-attrs probe",
    )


def _generate_dot_with_edge_side_effect_attrs() -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  { id: 'start', data: { label: 'Start', shape: 'Mdiamond' } },
  { id: 'route', data: { label: 'Route', shape: 'diamond', type: 'conditional', prompt: 'Decide route' } },
  { id: 'work', data: { label: 'Work', shape: 'box', type: 'codergen', prompt: 'Do work' } },
  { id: 'done', data: { label: 'Done', shape: 'Msquare' } }
]
const edges = [
  { id: 'e1', source: 'start', target: 'route' },
  {
    id: 'e2',
    source: 'route',
    target: 'work',
    data: {
      label: 'restart-work',
      condition: 'outcome=success && context.tests_passed=true',
      weight: '5',
      fidelity: 'summary:low',
      thread_id: 'edge-thread',
      loop_restart: true
    }
  },
  {
    id: 'e3',
    source: 'route',
    target: 'done',
    data: {
      label: 'skip'
    }
  },
  { id: 'e4', source: 'work', target: 'done' }
]
const dot = mod.generateDot('edge_side_effect_probe', nodes, edges, { default_fidelity: 'full' })
console.log(dot)
""".strip()
    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-edge-side-effects-",
        error_context="edge side effects probe",
    )


def test_edge_editor_exposes_all_required_edge_attrs_item_6_3_01() -> None:
    assert_frontend_behavior_contract_passed("6.3.01")


def test_edge_attrs_round_trip_through_preview_item_6_3_01() -> None:
    flow = _generate_dot_with_edge_attrs()
    payload = preview_pipeline(flow)
    edges = payload["graph"]["edges"]

    edge = next((candidate for candidate in edges if candidate["from"] == "start" and candidate["to"] == "route"), None)
    assert edge is not None
    assert edge["label"] == "success"
    assert edge["condition"] == "outcome=success && context.tests_passed=true"
    assert edge["weight"] == 7
    assert edge["fidelity"] == "summary:low"
    assert edge["thread_id"] == "review-thread"
    assert edge["loop_restart"] is True


def test_edge_condition_field_exposes_syntax_hints_and_preview_feedback_item_6_3_02() -> None:
    assert_frontend_behavior_contract_passed("6.3.02")


def test_edge_attr_serialization_and_execution_side_effect_visibility_item_6_3_03() -> None:
    flow = _generate_dot_with_edge_side_effect_attrs()
    payload = preview_pipeline(flow)
    edges = payload["graph"]["edges"]

    edge = next((candidate for candidate in edges if candidate["from"] == "route" and candidate["to"] == "work"), None)
    assert edge is not None
    assert edge["label"] == "restart-work"
    assert edge["condition"] == "outcome=success && context.tests_passed=true"
    assert edge["weight"] == 5
    assert edge["fidelity"] == "summary:low"
    assert edge["thread_id"] == "edge-thread"
    assert edge["loop_restart"] is True

    graph = parse_dot(flow)
    seen_runtime_true: list[str] = []

    def runner(node_id: str, prompt: str, context: Context) -> Outcome:
        del prompt, context
        seen_runtime_true.append(node_id)
        return Outcome(status=OutcomeStatus.SUCCESS)

    result_true = PipelineExecutor(graph, runner).run(Context(values={"tests_passed": True}))

    assert result_true.status == "success"
    assert seen_runtime_true == ["start", "route", "work"]
    assert result_true.route_trace == ["work", "done"]
    assert result_true.completed_nodes == ["work"]

    seen_runtime_false: list[str] = []

    def runner_false(node_id: str, prompt: str, context: Context) -> Outcome:
        del prompt, context
        seen_runtime_false.append(node_id)
        return Outcome(status=OutcomeStatus.SUCCESS)

    result_false = PipelineExecutor(graph, runner_false).run(Context(values={"tests_passed": False}))

    assert result_false.status == "success"
    assert seen_runtime_false == ["start", "route"]
    assert result_false.route_trace == ["start", "route", "done"]
    assert result_false.completed_nodes == ["start", "route"]
