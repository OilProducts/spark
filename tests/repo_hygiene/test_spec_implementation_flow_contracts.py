from __future__ import annotations

from pathlib import Path

from attractor.dsl import parse_dot


_SPEC_IMPLEMENTATION_DIR = (
    Path(__file__).resolve().parents[2] / "src" / "spark" / "flows" / "spec-implementation"
)


def _load_graph(dot_name: str):
    return parse_dot((_SPEC_IMPLEMENTATION_DIR / dot_name).read_text(encoding="utf-8"))


def _edge_targets(graph):
    return {
        (
            edge.source,
            edge.target,
            str(edge.attrs.get("label").value) if edge.attrs.get("label") is not None else "",
        )
        for edge in graph.edges
    }


def test_spec_implementation_milestone_worker_uses_validation_as_a_real_gate() -> None:
    graph = _load_graph("implement-milestone.dot")

    assert "assess_validation" in graph.nodes
    assert "gate_item_completion" in graph.nodes
    assert "validate_active_item_state" in graph.nodes

    validate_node = graph.nodes["validate_repo"]
    artifact_paths = str(validate_node.attrs["tool.artifacts.paths"].value)
    assert ".specflow/validation-plan.json" in artifact_paths
    assert ".specflow/validation-result.json" not in artifact_paths

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

    assert "validate_item_plan" in graph.nodes

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


def test_spec_implementation_milestone_worker_handles_partial_success_on_status_envelope_judgment_nodes() -> None:
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

    assert "commit_milestone" in graph.nodes

    commit_node = graph.nodes["commit_milestone"]
    assert str(commit_node.attrs["shape"].value) == "parallelogram"
    commit_command = str(commit_node.attrs["tool.command"].value)
    assert "git commit -m" in commit_command
    assert "Accepted milestone" in commit_command
    assert ".specflow/current-milestone.json" not in commit_command

    edge_targets = _edge_targets(graph)
    assert ("audit_milestone", "commit_milestone", "Commit") in edge_targets
    assert ("commit_milestone", "next_milestone", "Continue") in edge_targets
    assert ("commit_milestone", "blocked_exit", "Commit Failed") in edge_targets


def test_spec_implementation_parent_flow_validates_milestone_plan_before_dispatch() -> None:
    graph = _load_graph("implement-spec.dot")

    assert "validate_milestone_plan" in graph.nodes

    edge_targets = _edge_targets(graph)
    assert ("plan_milestones", "validate_milestone_plan", "") in edge_targets
    assert ("rewrite_milestones", "validate_milestone_plan", "") in edge_targets
    assert ("validate_milestone_plan", "next_milestone", "Validated") in edge_targets
    assert ("validate_milestone_plan", "plan_milestones", "Replan") in edge_targets
    assert ("plan_milestones", "next_milestone", "") not in edge_targets
    assert ("rewrite_milestones", "next_milestone", "") not in edge_targets


def test_spec_implementation_parent_flow_tracks_contract_decisions_through_architecture_and_milestones() -> None:
    graph = _load_graph("implement-spec.dot")

    design_prompt = str(graph.nodes["design_architecture"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in design_prompt
    assert "top-level decisions array" in design_prompt
    assert "behavioral_contract" in design_prompt
    assert "implementation_contract" in design_prompt
    assert "validation_expectations" in design_prompt

    evaluate_prompt = str(graph.nodes["evaluate_architecture"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in evaluate_prompt
    assert "silently narrow, rename, or reinterpret" in evaluate_prompt

    revise_prompt = str(graph.nodes["revise_architecture"].attrs["prompt"].value)
    assert "Update .specflow/contract-decisions.json" in revise_prompt

    plan_prompt = str(graph.nodes["plan_milestones"].attrs["prompt"].value)
    assert "decision_ids" in plan_prompt
    assert "Every relevant contract decision must appear" in plan_prompt

    validate_prompt = str(graph.nodes["validate_milestone_plan"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in validate_prompt
    assert "every relevant decision is covered" in validate_prompt
    assert "do not contradict their bound decisions" in validate_prompt

    next_prompt = str(graph.nodes["next_milestone"].attrs["prompt"].value)
    assert "context.milestone.decision_ids" in next_prompt
    writes_context = str(graph.nodes["next_milestone"].attrs["spark.writes_context"].value)
    assert "context.milestone.decision_ids" in writes_context


def test_spec_implementation_parent_flow_handles_partial_success_on_status_envelope_judgment_nodes() -> None:
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

    parent_writes = str(parent_graph.nodes["blocked_exit"].attrs["spark.writes_context"].value)
    child_writes = str(child_graph.nodes["blocked_exit"].attrs["spark.writes_context"].value)

    for writes_context in (parent_writes, child_writes):
        assert "context.workflow_outcome" in writes_context
        assert "context.workflow_outcome_reason_code" in writes_context
        assert "context.workflow_outcome_reason_message" in writes_context


def test_spec_implementation_parent_flow_automates_architecture_evaluation() -> None:
    graph = _load_graph("implement-spec.dot")

    assert "evaluate_architecture" in graph.nodes
    assert "review_architecture" not in graph.nodes

    edge_targets = _edge_targets(graph)
    assert ("design_architecture", "evaluate_architecture", "") in edge_targets
    assert ("evaluate_architecture", "plan_milestones", "Approve") in edge_targets
    assert ("evaluate_architecture", "revise_architecture", "Request Rework") in edge_targets
    assert ("evaluate_architecture", "blocked_exit", "Blocked") in edge_targets
    assert ("revise_architecture", "evaluate_architecture", "") in edge_targets


def test_spec_implementation_parent_flow_adds_final_repository_integrity_audit() -> None:
    graph = _load_graph("implement-spec.dot")

    assert "repository_integrity_audit" in graph.nodes
    assert "plan_cleanup_milestone" in graph.nodes

    edge_targets = _edge_targets(graph)
    assert ("final_conformance_audit", "repository_integrity_audit", "Requirements Covered") in edge_targets
    assert ("repository_integrity_audit", "plan_cleanup_milestone", "Cleanup") in edge_targets
    assert ("repository_integrity_audit", "blocked_exit", "Blocked") in edge_targets
    assert ("repository_integrity_audit", "done", "Complete") in edge_targets
    assert ("plan_cleanup_milestone", "validate_milestone_plan", "") in edge_targets
    assert ("audit_milestone", "revise_architecture", "Rework Architecture") not in edge_targets
    assert ("final_conformance_audit", "done", "Complete") not in edge_targets


def test_spec_implementation_flow_uses_gpt_54_for_judgment_heavy_nodes() -> None:
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
        assert parent_graph.nodes[node_id].attrs["llm_model"].value == "gpt-5.4"

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
        assert child_graph.nodes[node_id].attrs["llm_model"].value == "gpt-5.4"

    assert "llm_model" not in parent_graph.nodes["next_milestone"].attrs
    assert "llm_model" not in child_graph.nodes["implement_current"].attrs


def test_spec_implementation_child_flow_binds_items_to_contract_decisions() -> None:
    graph = _load_graph("implement-milestone.dot")

    prepare_state_reads = str(graph.nodes["prepare_milestone_state"].attrs["spark.reads_context"].value)
    assert "context.milestone.decision_ids" in prepare_state_reads

    prepare_state_prompt = str(graph.nodes["prepare_milestone_state"].attrs["prompt"].value)
    assert "bound context.milestone.* snapshot as authoritative" in prepare_state_prompt
    assert "repo-root .specflow/current-milestone.json later changes" in prepare_state_prompt

    extract_prompt = str(graph.nodes["extract_items"].attrs["prompt"].value)
    assert ".specflow/architecture.md" in extract_prompt
    assert ".specflow/contract-decisions.json" in extract_prompt
    assert "context.milestone.* plus milestone_dir/state.json as the authoritative child-worker milestone binding" in extract_prompt
    assert "decision_ids" in extract_prompt
    assert "rather than buried only in notes" in extract_prompt
    assert ".specflow/current-milestone.json" in extract_prompt

    next_prompt = str(graph.nodes["next_item"].attrs["prompt"].value)
    assert "context.item.decision_ids" in next_prompt
    next_writes = str(graph.nodes["next_item"].attrs["spark.writes_context"].value)
    assert "context.item.decision_ids" in next_writes
    assert "On the Audit path" in next_prompt

    mark_done_writes = str(graph.nodes["mark_current_done"].attrs["spark.writes_context"].value)
    assert "context.item.id" in mark_done_writes
    assert "context.validation.item_id" in mark_done_writes

    mark_blocked_writes = str(graph.nodes["mark_current_blocked"].attrs["spark.writes_context"].value)
    assert "context.item.id" in mark_blocked_writes
    assert "context.review.summary" in mark_blocked_writes

    rewrite_writes = str(graph.nodes["rewrite_current"].attrs["spark.writes_context"].value)
    assert "context.item.id" in rewrite_writes
    assert "context.validation.status" in rewrite_writes

    plan_prompt = str(graph.nodes["plan_current"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in plan_prompt
    assert "Convert the item's decision_ids into explicit implementation and validation goals" in plan_prompt
    plan_reads = str(graph.nodes["plan_current"].attrs["spark.reads_context"].value)
    assert "context.item.decision_ids" in plan_reads

    review_prompt = str(graph.nodes["review_current"].attrs["prompt"].value)
    assert ".specflow/architecture.md" in review_prompt
    assert ".specflow/contract-decisions.json" in review_prompt
    assert "weaker contract than the item's bound decisions" in review_prompt

    final_audit_prompt = str(graph.nodes["final_milestone_audit"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in final_audit_prompt
    assert "schema including decision_ids" in final_audit_prompt
    assert "Metadata-only repair items are allowed only when all bound decision_ids are already satisfied" in final_audit_prompt

    validate_prompt = str(graph.nodes["validate_item_plan"].attrs["prompt"].value)
    assert ".specflow/contract-decisions.json" in validate_prompt
    assert "every decision bound to the current milestone appears in at least one item's decision_ids" in validate_prompt
    assert "survives only in notes" in validate_prompt
    assert "do not use repo-root .specflow/current-milestone.json" in validate_prompt
    assert "treat success as a pre-dispatch queue state for next_item" in validate_prompt
    assert "next_item is solely responsible for dispatching work" in validate_prompt
    assert "stale context.item.*" in validate_prompt

    validate_writes = str(graph.nodes["validate_item_plan"].attrs["spark.writes_context"].value)
    assert "context.item.id" in validate_writes
    assert "context.review.summary" in validate_writes


def test_spec_implementation_flow_uses_bound_context_for_live_milestone_identity() -> None:
    parent_graph = _load_graph("implement-spec.dot")
    child_graph = _load_graph("implement-milestone.dot")

    child_description = str(child_graph.graph_attrs["spark.description"].value)
    assert "authoritative live active-item binding in protected context.item.* keys" in child_description
    assert "durable child queue ledger" in child_description
    assert "deliverable repository surfaces" in child_description

    blocked_prompt = str(child_graph.nodes["blocked_exit"].attrs["prompt"].value)
    assert "Treat context.milestone.id plus the milestone-local state as the authoritative identity" in blocked_prompt

    prepare_validation_prompt = str(child_graph.nodes["prepare_validation"].attrs["prompt"].value)
    assert "authoritative live binding for the running worker" in prepare_validation_prompt
    assert "durable ledger that must agree with them" in prepare_validation_prompt
    assert "Do not use repo-root .specflow/current-milestone.json or .specflow/state.json.current_milestone_id as the active milestone binding" in prepare_validation_prompt

    record_success_prompt = str(child_graph.nodes["record_milestone_success"].attrs["prompt"].value)
    assert "context.milestone.id plus milestone-local state as authoritative" in record_success_prompt

    final_audit_prompt = str(child_graph.nodes["final_milestone_audit"].attrs["prompt"].value)
    assert "deliverable repository surfaces" in final_audit_prompt

    audit_prompt = str(parent_graph.nodes["audit_milestone"].attrs["prompt"].value)
    assert "context.milestone.* plus milestone-local child artifacts as the authoritative identity" in audit_prompt
    assert "judge those files as repository deliverables" in audit_prompt

    rewrite_prompt = str(parent_graph.nodes["rewrite_milestones"].attrs["prompt"].value)
    assert "Treat context.milestone.id as the authoritative milestone to replace" in rewrite_prompt


def test_spec_implementation_flow_prompts_encode_repository_integrity_rubric() -> None:
    parent_graph = _load_graph("implement-spec.dot")
    child_graph = _load_graph("implement-milestone.dot")

    audit_prompt = str(parent_graph.nodes["audit_milestone"].attrs["prompt"].value)
    assert "honestly deliverable" in audit_prompt
    assert "test-only bootstrap hacks" in audit_prompt
    assert "outcome partial_success with preferred_label Extend" in audit_prompt

    integrity_prompt = str(parent_graph.nodes["repository_integrity_audit"].attrs["prompt"].value)
    assert ".specflow/repository-integrity-gaps.md" in integrity_prompt
    assert "tracked local, generated, cache, or IDE artifacts" in integrity_prompt
    assert "one additional cleanup/refactor milestone" in integrity_prompt

    cleanup_prompt = str(parent_graph.nodes["plan_cleanup_milestone"].attrs["prompt"].value)
    assert "empty requirement_ids list" in cleanup_prompt
    assert "empty decision_ids" in cleanup_prompt

    plan_prompt = str(child_graph.nodes["plan_current"].attrs["prompt"].value)
    assert "public bootstrap and installability" in plan_prompt

    implement_prompt = str(child_graph.nodes["implement_current"].attrs["prompt"].value)
    assert "Do not edit milestone_dir/state.json" in implement_prompt
    assert "Do not claim that a later item is now active" in implement_prompt

    prepare_validation_prompt = str(child_graph.nodes["prepare_validation"].attrs["prompt"].value)
    assert "fields item_id, item_title" in prepare_validation_prompt
    assert "ambient tooling" in prepare_validation_prompt
    assert "test-only path hacks" in prepare_validation_prompt
    assert "committed manifests" in prepare_validation_prompt
    assert "Do not edit milestone_dir/state.json in this node" in prepare_validation_prompt
    assert "Do not write .specflow/validation-result.json" in prepare_validation_prompt
    assert "ordinary repo-local files rather than symlinks or indirections outside the repository root" in prepare_validation_prompt

    assess_validation_prompt = str(child_graph.nodes["assess_validation"].attrs["prompt"].value)
    assert "fields item_id, item_title, status" in assess_validation_prompt
    assert "Return outcome success only when" in assess_validation_prompt
    assert "preferred_label Fix" in assess_validation_prompt

    review_prompt = str(child_graph.nodes["review_current"].attrs["prompt"].value)
    assert "Never return outcome success unless" in review_prompt
    assert "item_id matches context.item.id" in review_prompt
    assert "brittle source-text checks" in review_prompt
    assert "duplicate validators" in review_prompt
    assert "mixed-responsibility growth" in review_prompt

    active_item_prompt = str(child_graph.nodes["validate_active_item_state"].attrs["prompt"].value)
    assert "authoritative live binding for the active item" in active_item_prompt
    assert "state.json.current_item_id equals context.item.id" in active_item_prompt
    assert "preferred_label Blocked" in active_item_prompt

    validate_item_plan_prompt = str(child_graph.nodes["validate_item_plan"].attrs["prompt"].value)
    assert "at most one item is status=in_progress" in validate_item_plan_prompt
    assert "items with attempts=0 that are already in_progress or completed" in validate_item_plan_prompt
    assert "require current_item_id to be null" in validate_item_plan_prompt
    assert "do not promote any pending item to in_progress" in validate_item_plan_prompt
    assert "do not increment attempts" in validate_item_plan_prompt
    assert "active-item control-plane corruption" in validate_item_plan_prompt

    gate_prompt = str(child_graph.nodes["gate_item_completion"].attrs["prompt"].value)
    assert "state.json.current_item_id" in gate_prompt
    assert "preferred_label Blocked" in gate_prompt
    assert "preferred_label Revalidate" in gate_prompt

    mark_done_prompt = str(child_graph.nodes["mark_current_done"].attrs["prompt"].value)
    assert "Treat context.item.id as the only item" in mark_done_prompt
    assert "Do not modify any other item status" in mark_done_prompt
    assert "JSON null" in mark_done_prompt
    assert "preferred_label Blocked" in mark_done_prompt

    final_audit_prompt = str(child_graph.nodes["final_milestone_audit"].attrs["prompt"].value)
    assert "missing deliverability work" in final_audit_prompt

    next_item_prompt = str(child_graph.nodes["next_item"].attrs["prompt"].value)
    assert "cleared context.review.summary" in next_item_prompt
    assert "context.validation.item_id" in next_item_prompt
    assert "missing_prerequisites" in next_item_prompt
