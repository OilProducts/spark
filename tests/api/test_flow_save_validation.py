from __future__ import annotations

import json
import re
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from attractor.dsl import canonicalize_dot
from tests.contracts.frontend._support.dot_probe import run_canonical_flow_model_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


VALID_FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""

INVALID_PARSE_FLOW = """
digraph G {
    start [shape=Mdiamond]
    start -> done
"""

INVALID_VALIDATION_FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> missing
}
"""

SPEC_VALID_NO_OP_SAVE_FIXTURES: tuple[str, ...] = (
    "tests/fixtures/flows/reference-1.1-03-graph-attrs.dot",
    "starter-flows/manager-human.dot",
    "tests/fixtures/reference-1.1-03-subgraph-defaults.dot",
    "tests/fixtures/flows/reference-1.1-03-extension-attrs.dot",
)

ADVANCED_ATTR_ROUND_TRIP_FLOW = """
digraph AdvancedRoundTrip {
    start [shape=Mdiamond]
    task [
        shape=box,
        prompt="Do work",
        max_retries=0,
        goal_gate=false,
        retry_target="fix",
        fallback_retry_target="done",
        fidelity="summary:low",
        thread_id="thread-a",
        class="critical",
        timeout=900s,
        llm_model="gpt-5",
        llm_provider=openai,
        reasoning_effort=high,
        auto_status=false,
        allow_partial=false
    ]
    gate [
        shape=hexagon,
        prompt="Choose path",
        human.default_choice="fix"
    ]
    fix [shape=box, prompt="Fix issue"]
    done [shape=Msquare]

    start -> task
    task -> gate
    gate -> fix [label="Fix", loop_restart=false]
    gate -> done [label="Ship", weight=0]
    fix -> done
}
"""

LOSSY_LABEL_WITHOUT_SEMICOLON_FLOW = """
digraph LossyLabelWithoutSemicolon {
    start [shape=Mdiamond]
    task [
        shape=box,
        label="task",
        prompt="Do work",
        goal_gate=false,
        auto_status=false,
        allow_partial=false
    ]
    done [shape=Msquare]
    start -> task
    task -> done
}
"""


def _generate_no_op_save_candidate_from_preview(flow_name: str, flow_content: str) -> str:
    preview_payload = preview_pipeline(flow_content)
    preview_graph = preview_payload["graph"]
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.CANONICAL_FLOW_MODEL_JS_PATH).href)

const flowName = process.env.CANONICAL_ROUND_TRIP_FLOW_NAME ?? 'fixture.dot'
const previewGraph = JSON.parse(process.env.CANONICAL_ROUND_TRIP_PREVIEW_GRAPH ?? '{}')
const rawDot = process.env.CANONICAL_ROUND_TRIP_RAW_DOT ?? null
const model = mod.buildCanonicalFlowModelFromPreviewGraph(flowName, previewGraph, { rawDot })
const dot = mod.generateDotFromCanonicalFlowModel(flowName, model)
console.log(dot)
""".strip()
    return run_canonical_flow_model_probe(
        probe_script,
        temp_prefix=".tmp-canonical-no-op-save-",
        error_context=f"no-op semantic-equivalence probe for {flow_name}",
        env_extra={
            "CANONICAL_ROUND_TRIP_FLOW_NAME": flow_name,
            "CANONICAL_ROUND_TRIP_PREVIEW_GRAPH": json.dumps(preview_graph),
            "CANONICAL_ROUND_TRIP_RAW_DOT": flow_content,
        },
    )


def _generate_edited_advanced_attr_flow_from_preview(flow_name: str, flow_content: str) -> str:
    preview_payload = preview_pipeline(flow_content)
    preview_graph = preview_payload["graph"]
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.CANONICAL_FLOW_MODEL_JS_PATH).href)

const flowName = process.env.CANONICAL_ROUND_TRIP_FLOW_NAME ?? 'fixture.dot'
const previewGraph = JSON.parse(process.env.CANONICAL_ROUND_TRIP_PREVIEW_GRAPH ?? '{}')
const rawDot = process.env.CANONICAL_ROUND_TRIP_RAW_DOT ?? null
const model = mod.buildCanonicalFlowModelFromPreviewGraph(flowName, previewGraph, { rawDot })

const taskNode = model.nodes.find((node) => node.id === 'task')
if (taskNode) {
  taskNode.attrs.max_retries = 2
  taskNode.attrs.timeout = '120s'
  taskNode.attrs.llm_model = 'gpt-5.3'
  taskNode.attrs.reasoning_effort = 'medium'
  taskNode.attrs.goal_gate = false
  taskNode.attrs.auto_status = false
  taskNode.attrs.allow_partial = false
}

const gateToFixEdge = model.edges.find((edge) => edge.source === 'gate' && edge.target === 'fix')
if (gateToFixEdge) {
  gateToFixEdge.attrs.loop_restart = false
}

const dot = mod.generateDotFromCanonicalFlowModel(flowName, model)
console.log(dot)
""".strip()

    return run_canonical_flow_model_probe(
        probe_script,
        temp_prefix=".tmp-canonical-advanced-save-",
        error_context=f"advanced-attr round-trip probe for {flow_name}",
        env_extra={
            "CANONICAL_ROUND_TRIP_FLOW_NAME": flow_name,
            "CANONICAL_ROUND_TRIP_PREVIEW_GRAPH": json.dumps(preview_graph),
            "CANONICAL_ROUND_TRIP_RAW_DOT": flow_content,
        },
    )


def _generate_edited_label_regression_flow_from_preview(flow_name: str, flow_content: str) -> str:
    preview_payload = preview_pipeline(flow_content)
    preview_graph = preview_payload["graph"]
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.CANONICAL_FLOW_MODEL_JS_PATH).href)

const flowName = process.env.CANONICAL_ROUND_TRIP_FLOW_NAME ?? 'fixture.dot'
const previewGraph = JSON.parse(process.env.CANONICAL_ROUND_TRIP_PREVIEW_GRAPH ?? '{}')
const rawDot = process.env.CANONICAL_ROUND_TRIP_RAW_DOT ?? null
const model = mod.buildCanonicalFlowModelFromPreviewGraph(flowName, previewGraph, { rawDot })

const taskNode = model.nodes.find((node) => node.id === 'task')
if (taskNode) {
  taskNode.attrs.timeout = '45s'
}

const dot = mod.generateDotFromCanonicalFlowModel(flowName, model)
console.log(dot)
""".strip()

    return run_canonical_flow_model_probe(
        probe_script,
        temp_prefix=".tmp-canonical-lossy-label-regression-",
        error_context=f"lossy-label regression probe for {flow_name}",
        env_extra={
            "CANONICAL_ROUND_TRIP_FLOW_NAME": flow_name,
            "CANONICAL_ROUND_TRIP_PREVIEW_GRAPH": json.dumps(preview_graph),
            "CANONICAL_ROUND_TRIP_RAW_DOT": flow_content,
        },
    )


def test_save_flow_rejects_parse_invalid_dot(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    target = tmp_path / "flows" / "bad.dot"

    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "bad.dot",
            "content": INVALID_PARSE_FLOW,
        },
    )

    assert response.status_code == 422
    detail = response.json()["detail"]
    assert detail["status"] == "parse_error"
    assert detail["errors"], "parse failures must return actionable errors"
    assert not target.exists()


def test_save_flow_rejects_validation_error_dot(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    target = tmp_path / "flows" / "bad.dot"

    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "bad.dot",
            "content": INVALID_VALIDATION_FLOW,
        },
    )

    assert response.status_code == 422
    detail = response.json()["detail"]
    assert detail["status"] == "validation_error"
    assert detail["diagnostics"], "validation failures must include diagnostics"
    assert detail["errors"], "validation failures must include error diagnostics"
    assert not target.exists()


def test_save_flow_persists_valid_dot(api_client: TestClient, tmp_path: Path) -> None:
    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "good.dot",
            "content": VALID_FLOW,
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "saved"
    assert payload["name"] == "good.dot"
    assert (tmp_path / "flows" / "good.dot").read_text(encoding="utf-8") == canonicalize_dot(
        VALID_FLOW
    )


def test_save_flow_reports_semantic_equivalence_for_no_behavior_change(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(exist_ok=True)
    (flows_dir / "good.dot").write_text(
        """
digraph ExistingFlow {
    done [shape=Msquare]
    start [shape=Mdiamond]
    start -> done
}
""",
        encoding="utf-8",
    )

    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "good.dot",
            "content": VALID_FLOW,
            "expect_semantic_equivalence": True,
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "saved"
    assert payload["semantic_equivalent_to_existing"] is True


def test_save_flow_rejects_semantic_mismatch_when_equivalence_is_expected(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(exist_ok=True)
    target = flows_dir / "good.dot"
    target.write_text(VALID_FLOW, encoding="utf-8")

    non_equivalent_flow = """
digraph G {
    start [shape=Mdiamond]
    review [shape=box, prompt="Review intermediate output"]
    done [shape=Msquare]
    start -> review
    review -> done
}
"""

    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "good.dot",
            "content": non_equivalent_flow,
            "expect_semantic_equivalence": True,
        },
    )

    assert response.status_code == 409
    detail = response.json()["detail"]
    assert detail["status"] == "semantic_mismatch"
    assert target.read_text(encoding="utf-8") == VALID_FLOW


@pytest.mark.parametrize(
    "fixture_rel_path",
    SPEC_VALID_NO_OP_SAVE_FIXTURES,
    ids=lambda rel_path: Path(rel_path).name,
)
def test_save_flow_no_op_semantic_equivalence_for_spec_valid_fixtures_item_11_2_01(
    api_client: TestClient,
    fixture_rel_path: str,
) -> None:
    repo_root = Path(__file__).resolve().parents[2]
    fixture_content = (repo_root / fixture_rel_path).read_text(encoding="utf-8")
    fixture_flow_name = f"fixture-no-op-{Path(fixture_rel_path).stem}.dot"

    seed_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": fixture_flow_name,
            "content": fixture_content,
        },
    )
    assert seed_response.status_code == 200, seed_response.text

    no_op_save_content = _generate_no_op_save_candidate_from_preview(
        fixture_flow_name,
        fixture_content,
    )

    no_op_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": fixture_flow_name,
            "content": no_op_save_content,
            "expect_semantic_equivalence": True,
        },
    )
    assert no_op_response.status_code == 200, no_op_response.text
    no_op_payload = no_op_response.json()
    assert no_op_payload["status"] == "saved"
    assert no_op_payload["semantic_equivalent_to_existing"] is True


def test_save_flow_open_edit_save_reopen_preserves_advanced_attrs_item_11_2_02(
    api_client: TestClient,
) -> None:
    flow_name = "fixture-open-edit-save-reopen-advanced.dot"
    seed_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": flow_name,
            "content": ADVANCED_ATTR_ROUND_TRIP_FLOW,
        },
    )
    assert seed_response.status_code == 200, seed_response.text

    edited_content = _generate_edited_advanced_attr_flow_from_preview(
        flow_name,
        ADVANCED_ATTR_ROUND_TRIP_FLOW,
    )
    assert "auto_status=false" in edited_content
    assert "allow_partial=false" in edited_content
    assert "goal_gate=false" in edited_content
    assert "loop_restart=false" in edited_content

    save_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": flow_name,
            "content": edited_content,
        },
    )
    assert save_response.status_code == 200, save_response.text

    reopen_response = api_client.get(f"/attractor/api/flows/{flow_name}")
    assert reopen_response.status_code == 200, reopen_response.text
    reopened_content = reopen_response.json()["content"]

    reopened_payload = preview_pipeline(reopened_content)
    nodes = reopened_payload["graph"]["nodes"]
    edges = reopened_payload["graph"]["edges"]
    task_node = next((node for node in nodes if node["id"] == "task"), None)
    gate_to_fix = next(
        (
            edge
            for edge in edges
            if edge["from"] == "gate" and edge["to"] == "fix"
        ),
        None,
    )
    assert task_node is not None
    assert gate_to_fix is not None

    assert task_node["max_retries"] == 2
    assert task_node["timeout"] == "120s"
    assert task_node["llm_model"] == "gpt-5.3"
    assert task_node["reasoning_effort"] == "medium"
    assert task_node["goal_gate"] is False
    assert task_node["auto_status"] is False
    assert task_node["allow_partial"] is False
    assert gate_to_fix["loop_restart"] is False


def test_save_flow_open_edit_save_reopen_preserves_label_regressions_item_11_2_03(
    api_client: TestClient,
) -> None:
    flow_name = "fixture-open-edit-save-reopen-lossy-label.dot"
    seed_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": flow_name,
            "content": LOSSY_LABEL_WITHOUT_SEMICOLON_FLOW,
        },
    )
    assert seed_response.status_code == 200, seed_response.text

    edited_content = _generate_edited_label_regression_flow_from_preview(
        flow_name,
        LOSSY_LABEL_WITHOUT_SEMICOLON_FLOW,
    )
    assert "timeout=45s" in edited_content

    save_response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": flow_name,
            "content": edited_content,
        },
    )
    assert save_response.status_code == 200, save_response.text

    reopen_response = api_client.get(f"/attractor/api/flows/{flow_name}")
    assert reopen_response.status_code == 200, reopen_response.text
    reopened_content = reopen_response.json()["content"]

    assert re.search(r"\btask\s*\[[^\]]*\blabel=\"task\"", reopened_content), reopened_content

    reopened_payload = preview_pipeline(reopened_content)
    task_node = next((node for node in reopened_payload["graph"]["nodes"] if node["id"] == "task"), None)
    assert task_node is not None
    assert task_node["timeout"] == "45s"
    assert task_node["goal_gate"] is False
    assert task_node["auto_status"] is False
    assert task_node["allow_partial"] is False


def test_flow_endpoints_use_project_root_not_process_cwd(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    unrelated_cwd = tmp_path / "other-cwd"
    unrelated_cwd.mkdir(parents=True, exist_ok=True)
    monkeypatch.chdir(unrelated_cwd)

    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "cwd-check.dot",
            "content": VALID_FLOW,
        },
    )
    assert response.status_code == 200

    assert (tmp_path / "flows" / "cwd-check.dot").exists()
    assert not (unrelated_cwd / "flows" / "cwd-check.dot").exists()


def test_get_flow_raises_404_for_missing_flow(api_client: TestClient) -> None:
    response = api_client.get("/attractor/api/flows/missing.dot")

    assert response.status_code == 404
    assert response.json()["detail"] == "Flow not found."


def test_delete_flow_deletes_existing_flow_and_raises_404_for_missing(
    api_client: TestClient,
    tmp_path: Path,
) -> None:
    flow_path = tmp_path / "flows" / "delete-me.dot"
    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text(VALID_FLOW, encoding="utf-8")

    delete_response = api_client.delete("/attractor/api/flows/delete-me.dot")
    assert delete_response.status_code == 200
    payload = delete_response.json()
    assert payload == {"status": "deleted"}
    assert not flow_path.exists()

    missing_response = api_client.delete("/attractor/api/flows/delete-me.dot")
    assert missing_response.status_code == 404
    assert missing_response.json()["detail"] == "Flow not found."


def test_flow_name_must_be_single_file_name(api_client: TestClient) -> None:
    response = api_client.post(
        "/attractor/api/flows",
        json={
            "name": "../escape.dot",
            "content": VALID_FLOW,
        },
    )

    assert response.status_code == 400
    assert response.json()["detail"] == "Flow name must be a single file name."
