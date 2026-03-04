from __future__ import annotations

import json
from pathlib import Path

from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_contract_passed
from tests.contracts.frontend._support.dot_probe import run_dot_utils_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def _generate_dot_with_manager_loop_attrs() -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  {
    id: 'manager',
    data: {
      label: 'Manager',
      shape: 'house',
      type: 'stack.manager_loop',
      'manager.poll_interval': '25ms',
      'manager.max_cycles': '3',
      'manager.stop_condition': 'child.status == "success"',
      'manager.actions': 'observe,steer'
    }
  }
]
const dot = mod.generateDot('manager_loop_probe', nodes, [], {})
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-manager-loop-",
        error_context="manager-loop probe",
    )


def _generate_dot_from_preview_graph(graph_payload: dict[str, object]) -> str:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const payload = JSON.parse(process.env.PREVIEW_GRAPH_JSON)
const nodes = payload.nodes.map((node) => {
  const { id, ...attrs } = node
  return { id, data: attrs }
})
const edges = payload.edges.map((edge) => {
  const { source, target, ...attrs } = edge
  return { source, target, data: attrs }
})
const dot = mod.generateDot('manager_loop_fixture_round_trip_probe', nodes, edges, payload.graph_attrs ?? {})
console.log(dot)
""".strip()

    return run_dot_utils_probe(
        probe_script,
        temp_prefix=".tmp-dotutils-manager-loop-roundtrip-",
        error_context="manager-loop fixture round-trip probe",
        env_extra={"PREVIEW_GRAPH_JSON": json.dumps(graph_payload)},
    )


def test_manager_loop_authoring_controls_present_item_6_2_01() -> None:
    assert_frontend_behavior_contract_passed("6.2.01")


def test_manager_loop_shape_and_type_are_selectable_item_6_7_01() -> None:
    assert_frontend_behavior_contract_passed("6.7.01")


def test_manager_loop_control_fields_present_item_6_7_02() -> None:
    assert_frontend_behavior_contract_passed("6.7.02")


def test_manager_loop_child_linkage_controls_present_item_6_7_03() -> None:
    assert_frontend_behavior_contract_passed("6.7.03")


def test_manager_loop_attrs_round_trip_through_preview_item_6_2_01() -> None:
    flow = _generate_dot_with_manager_loop_attrs()
    payload = preview_pipeline(flow)
    nodes = payload["graph"]["nodes"]
    manager_node = next((node for node in nodes if node["id"] == "manager"), None)

    assert manager_node is not None
    assert manager_node["shape"] == "house"
    assert manager_node["type"] == "stack.manager_loop"
    assert manager_node["manager.poll_interval"] == "25ms"
    assert manager_node["manager.max_cycles"] == 3
    assert manager_node["manager.stop_condition"] == 'child.status == "success"'
    assert manager_node["manager.actions"] == "observe,steer"


def test_manager_loop_fixture_round_trip_item_6_7_04() -> None:
    repo_root = Path(__file__).resolve().parents[3]
    fixture_path = repo_root / "tests" / "fixtures" / "manager_loop_authoring_round_trip.dot"
    assert fixture_path.exists(), f"Missing manager-loop fixture: {fixture_path}"

    fixture_payload = preview_pipeline(fixture_path.read_text(encoding="utf-8"))
    round_trip_flow = _generate_dot_from_preview_graph(fixture_payload["graph"])
    round_trip_payload = preview_pipeline(round_trip_flow)

    fixture_graph_attrs = fixture_payload["graph"]["graph_attrs"]
    round_trip_graph_attrs = round_trip_payload["graph"]["graph_attrs"]
    assert round_trip_graph_attrs["stack.child_dotfile"] == fixture_graph_attrs["stack.child_dotfile"]
    assert round_trip_graph_attrs["stack.child_workdir"] == fixture_graph_attrs["stack.child_workdir"]

    fixture_manager = next(node for node in fixture_payload["graph"]["nodes"] if node["id"] == "manager")
    round_trip_manager = next(node for node in round_trip_payload["graph"]["nodes"] if node["id"] == "manager")
    for key in (
        "shape",
        "type",
        "manager.poll_interval",
        "manager.max_cycles",
        "manager.stop_condition",
        "manager.actions",
    ):
        assert round_trip_manager[key] == fixture_manager[key]
