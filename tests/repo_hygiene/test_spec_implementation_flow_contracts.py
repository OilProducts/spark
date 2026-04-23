from __future__ import annotations

import json
from pathlib import Path

from attractor.dsl import parse_dot


_SPEC_IMPLEMENTATION_DIR = (
    Path(__file__).resolve().parents[2]
    / "src"
    / "spark"
    / "flows"
    / "software-development"
    / "spec-implementation"
)


def _load_graph(dot_name: str):
    return parse_dot((_SPEC_IMPLEMENTATION_DIR / dot_name).read_text(encoding="utf-8"))


def _source(dot_name: str) -> str:
    return (_SPEC_IMPLEMENTATION_DIR / dot_name).read_text(encoding="utf-8")


def _edge_targets(graph):
    return {
        (
            edge.source,
            edge.target,
            str(edge.attrs.get("label").value) if edge.attrs.get("label") is not None else "",
        )
        for edge in graph.edges
    }


def _json_array_attr(graph, node_id: str, attr_name: str) -> set[str]:
    attr = graph.nodes[node_id].attrs.get(attr_name)
    assert attr is not None, f"missing {attr_name} for {node_id}"
    parsed = json.loads(str(attr.value))
    assert isinstance(parsed, list)
    return {str(item) for item in parsed}


def test_spec_implementation_flows_use_new_runtime_and_durable_namespaces() -> None:
    parent_source = _source("implement-spec.dot")
    child_source = _source("implement-milestone.dot")

    for source in (parent_source, child_source):
        assert ".specflow/" not in source
        assert ".spark/spec-implementation/" in source

    assert "specs/<slug>/" in parent_source
    assert ".spark/spec-implementation/current/spec/" in parent_source
    assert ".spark/spec-implementation/current/spec/" in child_source


def test_spec_implementation_milestone_worker_uses_validation_as_a_real_gate() -> None:
    graph = _load_graph("implement-milestone.dot")

    assert "assess_validation" in graph.nodes
    assert "gate_item_completion" in graph.nodes
    assert "validate_active_item_state" in graph.nodes

    validate_node = graph.nodes["validate_repo"]
    artifact_paths = str(validate_node.attrs["tool.artifacts.paths"].value)
    assert ".spark/spec-implementation/current/validation-plan.json" in artifact_paths
    assert ".spark/spec-implementation/current/validation-result.json" not in artifact_paths

    edge_targets = _edge_targets(graph)
    assert ("validate_repo", "assess_validation", "Assess Pass") in edge_targets
    assert ("validate_repo", "assess_validation", "Assess Fail") in edge_targets
    assert ("assess_validation", "review_current", "Review") in edge_targets
    assert ("assess_validation", "implement_current", "Fix Validation") in edge_targets
    assert ("assess_validation", "rewrite_current", "Rewrite Validation") in edge_targets
    assert ("assess_validation", "mark_current_blocked", "Blocked Validation") in edge_targets
    assert ("implement_current", "validate_active_item_state", "Check Item State") in edge_targets
    assert ("validate_active_item_state", "prepare_validation", "Validate") in edge_targets
    assert ("validate_active_item_state", "blocked_exit", "Blocked State") in edge_targets
    assert ("review_current", "gate_item_completion", "Ready To Complete") in edge_targets
    assert ("gate_item_completion", "blocked_exit", "Blocked State") in edge_targets
    assert ("gate_item_completion", "mark_current_done", "Validated") in edge_targets
    assert ("gate_item_completion", "prepare_validation", "Revalidate") in edge_targets
    assert ("mark_current_blocked", "blocked_exit", "Blocked State") in edge_targets
    assert ("mark_current_done", "blocked_exit", "Blocked State") in edge_targets
    assert ("rewrite_current", "blocked_exit", "Blocked Rewrite") in edge_targets
    assert ("review_current", "mark_current_done", "Done") not in edge_targets
    assert ("implement_current", "prepare_validation", "Validate") not in edge_targets


def test_spec_implementation_milestone_worker_validates_item_queue_before_next_item() -> None:
    graph = _load_graph("implement-milestone.dot")

    edge_targets = _edge_targets(graph)
    assert ("extract_items", "validate_item_plan", "") in edge_targets
    assert ("rewrite_current", "validate_item_plan", "") in edge_targets
    assert ("final_milestone_audit", "validate_item_plan", "Extend") in edge_targets
    assert ("validate_item_plan", "next_item", "Validated") in edge_targets
    assert ("validate_item_plan", "blocked_exit", "Blocked") in edge_targets
    assert ("validate_item_plan", "extract_items", "Reextract") in edge_targets
    assert ("extract_items", "next_item", "") not in edge_targets
    assert ("rewrite_current", "next_item", "") not in edge_targets
    assert ("final_milestone_audit", "next_item", "Extend") not in edge_targets


def test_spec_implementation_milestone_worker_handles_partial_success_routes() -> None:
    graph = _load_graph("implement-milestone.dot")

    edge_targets = _edge_targets(graph)
    assert ("assess_validation", "implement_current", "Fix Validation Partial") in edge_targets
    assert ("assess_validation", "mark_current_blocked", "Blocked Validation Partial") in edge_targets
    assert ("assess_validation", "rewrite_current", "Rewrite Validation Partial") in edge_targets
    assert ("blocked_exit", "done", "Finish Partial Blocked") in edge_targets
    assert ("final_milestone_audit", "validate_item_plan", "Extend Partial") in edge_targets
    assert ("final_milestone_audit", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("gate_item_completion", "prepare_validation", "Revalidate Partial") in edge_targets
    assert ("gate_item_completion", "blocked_exit", "Blocked State Partial") in edge_targets
    assert ("next_item", "plan_current", "Work Partial") in edge_targets
    assert ("next_item", "final_milestone_audit", "Audit Partial") in edge_targets
    assert ("next_item", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("review_current", "implement_current", "Fix Partial") in edge_targets
    assert ("review_current", "mark_current_blocked", "Block Partial") in edge_targets
    assert ("review_current", "rewrite_current", "Rewrite Partial") in edge_targets
    assert ("validate_active_item_state", "blocked_exit", "Blocked State Partial") in edge_targets
    assert ("mark_current_blocked", "blocked_exit", "Blocked State Partial") in edge_targets
    assert ("rewrite_current", "blocked_exit", "Blocked Rewrite Partial") in edge_targets
    assert ("validate_item_plan", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("validate_item_plan", "extract_items", "Reextract Partial") in edge_targets


def test_spec_implementation_parent_flow_commits_after_milestone_acceptance() -> None:
    graph = _load_graph("implement-spec.dot")

    commit_node = graph.nodes["commit_milestone"]
    assert str(commit_node.attrs["shape"].value) == "parallelogram"
    commit_command = str(commit_node.attrs["tool.command"].value)
    assert "git add -A" in commit_command
    assert "git commit -m \"Accepted milestone\"" in commit_command

    edge_targets = _edge_targets(graph)
    assert ("audit_milestone", "commit_milestone", "Commit") in edge_targets
    assert ("commit_milestone", "next_milestone", "Continue") in edge_targets
    assert ("commit_milestone", "blocked_exit", "Commit Failed") in edge_targets


def test_spec_implementation_parent_flow_validates_milestone_plan_before_dispatch() -> None:
    graph = _load_graph("implement-spec.dot")

    edge_targets = _edge_targets(graph)
    assert ("plan_milestones", "validate_milestone_plan", "") in edge_targets
    assert ("rewrite_milestones", "validate_milestone_plan", "") in edge_targets
    assert ("validate_milestone_plan", "next_milestone", "Validated") in edge_targets
    assert ("validate_milestone_plan", "plan_milestones", "Replan") in edge_targets
    assert ("plan_milestones", "next_milestone", "") not in edge_targets
    assert ("rewrite_milestones", "next_milestone", "") not in edge_targets


def test_spec_implementation_parent_flow_exposes_spec_slug_and_decision_context() -> None:
    graph = _load_graph("implement-spec.dot")

    launch_inputs = json.loads(str(graph.graph_attrs["spark.launch_inputs"].value))
    launch_input_keys = {str(item["key"]) for item in launch_inputs}
    assert "context.request.spec_slug" in launch_input_keys

    next_writes = _json_array_attr(graph, "next_milestone", "spark.writes_context")
    assert {
        "context.milestone.id",
        "context.milestone.title",
        "context.milestone.objective",
        "context.milestone.requirement_ids",
        "context.milestone.decision_ids",
        "context.milestone.acceptance_criteria",
        "context.milestone.target_paths",
        "context.milestone.attempts",
    } <= next_writes

    audit_reads = _json_array_attr(graph, "audit_milestone", "spark.reads_context")
    assert {
        "context.milestone.id",
        "context.milestone.decision_ids",
        "context.stack.child.status",
        "context.stack.child.outcome",
        "context.stack.child.outcome_reason_code",
        "context.stack.child.outcome_reason_message",
    } <= audit_reads


def test_spec_implementation_parent_flow_handles_partial_success_routes() -> None:
    graph = _load_graph("implement-spec.dot")

    edge_targets = _edge_targets(graph)
    assert ("audit_milestone", "run_milestone", "Extend") in edge_targets
    assert ("audit_milestone", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("audit_milestone", "rewrite_milestones", "Rewrite Milestones Partial") in edge_targets
    assert ("blocked_exit", "done", "Finish Partial Blocked") in edge_targets
    assert ("evaluate_architecture", "revise_architecture", "Request Partial Rework") in edge_targets
    assert ("evaluate_architecture", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("final_conformance_audit", "plan_milestones", "Extend Partial") in edge_targets
    assert ("final_conformance_audit", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("next_milestone", "run_milestone", "Work Partial") in edge_targets
    assert ("next_milestone", "final_conformance_audit", "Audit Partial") in edge_targets
    assert ("next_milestone", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("repository_integrity_audit", "plan_cleanup_milestone", "Cleanup Partial") in edge_targets
    assert ("repository_integrity_audit", "blocked_exit", "Blocked Partial") in edge_targets
    assert ("validate_milestone_plan", "plan_milestones", "Replan Partial") in edge_targets


def test_spec_implementation_blocked_exit_declares_workflow_outcome_writes() -> None:
    parent_graph = _load_graph("implement-spec.dot")
    child_graph = _load_graph("implement-milestone.dot")

    for graph in (parent_graph, child_graph):
        writes_context = _json_array_attr(graph, "blocked_exit", "spark.writes_context")
        assert {
            "context.workflow_outcome",
            "context.workflow_outcome_reason_code",
            "context.workflow_outcome_reason_message",
        } <= writes_context


def test_spec_implementation_parent_flow_automates_architecture_evaluation() -> None:
    graph = _load_graph("implement-spec.dot")

    edge_targets = _edge_targets(graph)
    assert ("design_architecture", "evaluate_architecture", "") in edge_targets
    assert ("evaluate_architecture", "plan_milestones", "Approve") in edge_targets
    assert ("evaluate_architecture", "revise_architecture", "Request Rework") in edge_targets
    assert ("evaluate_architecture", "blocked_exit", "Blocked") in edge_targets
    assert ("revise_architecture", "evaluate_architecture", "") in edge_targets
    assert "review_architecture" not in graph.nodes


def test_spec_implementation_parent_flow_adds_final_repository_integrity_audit() -> None:
    graph = _load_graph("implement-spec.dot")

    edge_targets = _edge_targets(graph)
    assert ("final_conformance_audit", "repository_integrity_audit", "Requirements Covered") in edge_targets
    assert ("repository_integrity_audit", "plan_cleanup_milestone", "Cleanup") in edge_targets
    assert ("repository_integrity_audit", "blocked_exit", "Blocked") in edge_targets
    assert ("repository_integrity_audit", "done", "Complete") in edge_targets
    assert ("plan_cleanup_milestone", "validate_milestone_plan", "") in edge_targets
    assert ("final_conformance_audit", "done", "Complete") not in edge_targets


def test_spec_implementation_flow_uses_expected_models_for_judgment_nodes() -> None:
    parent_graph = _load_graph("implement-spec.dot")
    child_graph = _load_graph("implement-milestone.dot")

    assert parent_graph.graph_attrs["ui_default_llm_model"].value == "gpt-5.4-mini"
    assert parent_graph.graph_attrs["ui_default_reasoning_effort"].value == "high"
    assert child_graph.graph_attrs["ui_default_llm_model"].value == "gpt-5.4-mini"
    assert child_graph.graph_attrs["ui_default_reasoning_effort"].value == "high"

    parent_gpt_54_nodes = {
        "extract_requirements",
        "design_architecture",
        "evaluate_architecture",
        "revise_architecture",
        "plan_milestones",
        "validate_milestone_plan",
        "audit_milestone",
        "final_conformance_audit",
        "repository_integrity_audit",
        "plan_cleanup_milestone",
    }
    for node_id in parent_gpt_54_nodes:
        assert parent_graph.nodes[node_id].attrs["llm_model"].value == "gpt-5.5"

    child_gpt_54_nodes = {
        "extract_items",
        "plan_current",
        "prepare_validation",
        "assess_validation",
        "review_current",
        "gate_item_completion",
        "validate_active_item_state",
        "final_milestone_audit",
        "validate_item_plan",
    }
    for node_id in child_gpt_54_nodes:
        assert child_graph.nodes[node_id].attrs["llm_model"].value == "gpt-5.5"

    assert "llm_model" not in parent_graph.nodes["next_milestone"].attrs
    assert "llm_model" not in child_graph.nodes["implement_current"].attrs


def test_spec_implementation_child_flow_preserves_live_item_context_contracts() -> None:
    graph = _load_graph("implement-milestone.dot")

    prepare_state_reads = _json_array_attr(graph, "prepare_milestone_state", "spark.reads_context")
    assert "context.milestone.decision_ids" in prepare_state_reads

    next_writes = _json_array_attr(graph, "next_item", "spark.writes_context")
    assert {
        "context.item.id",
        "context.item.decision_ids",
        "context.review.summary",
        "context.validation.item_id",
        "missing_prerequisites",
        "validation_message",
        "validation_exit_code",
    } <= next_writes

    mark_done_writes = _json_array_attr(graph, "mark_current_done", "spark.writes_context")
    assert {
        "context.item.id",
        "context.validation.item_id",
        "context.review.summary",
    } <= mark_done_writes

    mark_blocked_writes = _json_array_attr(graph, "mark_current_blocked", "spark.writes_context")
    assert {
        "context.item.id",
        "context.review.summary",
        "context.validation.status",
    } <= mark_blocked_writes

    rewrite_writes = _json_array_attr(graph, "rewrite_current", "spark.writes_context")
    assert {
        "context.item.id",
        "context.validation.status",
        "context.review.required_changes",
    } <= rewrite_writes

    validate_plan_writes = _json_array_attr(graph, "validate_item_plan", "spark.writes_context")
    assert {
        "context.item.id",
        "context.review.summary",
        "context.validation.status",
    } <= validate_plan_writes

    plan_reads = _json_array_attr(graph, "plan_current", "spark.reads_context")
    assert "context.item.decision_ids" in plan_reads


def test_spec_implementation_runtime_validation_tool_paths_live_under_spark_namespace() -> None:
    parent_graph = _load_graph("implement-spec.dot")
    child_graph = _load_graph("implement-milestone.dot")

    assert parent_graph.graph_attrs["stack.child_dotfile"].value == "implement-milestone.dot"

    validate_node = child_graph.nodes["validate_repo"]
    command = str(validate_node.attrs["tool.command"].value)
    assert command == "bash .spark/spec-implementation/current/run-milestone-validation.sh"

    artifact_paths = set(str(validate_node.attrs["tool.artifacts.paths"].value).split(","))
    assert {
        ".spark/spec-implementation/current/run-milestone-validation.sh",
        ".spark/spec-implementation/current/validation-plan.json",
        ".spark/spec-implementation/current/validation.stdout.txt",
        ".spark/spec-implementation/current/validation.stderr.txt",
    } <= artifact_paths
