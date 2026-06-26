from __future__ import annotations

import json
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
RECORD_PATH = REPO_ROOT / ".spark" / "rust-rewrite" / "current" / "migration-records.json"
COMPAT_FIXTURE_ROOT = REPO_ROOT / ".spark" / "rust-rewrite" / "current" / "compat-fixtures"

ITEM_ID = "M7-I03-MIGRATION-ADAPTER-NONGOAL-RECORDS"
MILESTONE_ID = "M7-DOCS-VALIDATION"
BOUND_REQUIREMENTS = {"RR-DOC-003", "RR-VAL-001", "RR-VAL-002", "RR-VAL-005"}
BOUND_DECISIONS = {"CD-RR-001", "CD-RR-013", "CD-RR-014", "CD-RR-015"}
ALLOWED_CLASSIFICATIONS = {
    "native_rust",
    "rust_owned_adapter",
    "retained_python_module",
}


def test_migration_record_is_bound_to_active_item_requirements_and_decisions() -> None:
    record = _load_record()

    assert record["schema_version"] == "rust-rewrite-migration-records-v1"
    assert record["item_id"] == ITEM_ID
    assert record["milestone_id"] == MILESTONE_ID
    assert set(record["requirements"]) == BOUND_REQUIREMENTS
    assert set(record["decisions"]) == BOUND_DECISIONS
    assert set(record["classification_values"]) == ALLOWED_CLASSIFICATIONS
    assert record["behavior_change"] == "none"

    change_record_id = record["change_record_id"]
    change_record = REPO_ROOT / "changes" / change_record_id
    assert change_record.joinpath("request.md").is_file()
    assert change_record.joinpath("result.md").is_file()


def test_agent_and_unified_llm_records_classify_each_boundary_with_evidence() -> None:
    record = _load_record()
    surfaces = {surface["surface_id"]: surface for surface in record["surface_records"]}

    assert set(surfaces) == {"agent", "unified_llm"}
    for surface in surfaces.values():
        boundaries = surface["boundaries"]
        assert boundaries
        classifications = {boundary["classification"] for boundary in boundaries}
        assert classifications <= ALLOWED_CLASSIFICATIONS
        assert "retained_python_module" in classifications
        assert classifications & {"native_rust", "rust_owned_adapter"}
        for boundary in boundaries:
            assert boundary["id"]
            assert boundary["current_owner"]
            assert boundary["retained_python_status"]
            assert boundary["observable_contracts"]
            _assert_evidence(boundary["evidence"])


def test_deprecated_surfaces_are_preserved_until_future_contract_decision() -> None:
    record = _load_record()
    surfaces = {surface["id"]: surface for surface in record["deprecated_surfaces"]}

    assert {
        "attractor_runs_events",
        "workspace_conversation_events",
        "attractor_pipeline_events",
    } <= set(surfaces)

    for surface in surfaces.values():
        assert surface["status"] == "preserved_compatibility"
        assert surface["replacement_surface"]
        assert surface["removal_allowed_without_contract_decision"] is False
        assert surface["requires_new_contract_decision"] is True
        _assert_evidence(surface["evidence"])


def test_policy_gaps_and_non_goals_are_not_counted_as_closed_parity() -> None:
    record = _load_record()
    policy_gaps = {entry["id"]: entry for entry in record["policy_gaps"]}
    non_goals = {entry["id"]: entry for entry in record["explicit_non_goals"]}

    assert {
        "acceptance_workflow_harness",
    } <= set(policy_gaps)
    assert "manager_loop_telemetry_ingestion" not in policy_gaps
    assert {
        "full_agent_python_removal_in_m7",
        "full_unified_llm_python_removal_in_m7",
        "acceptance_workflow_harness_closure_in_m7",
        "remote_worker_reintroduction",
    } <= set(non_goals)

    for entry in [*policy_gaps.values(), *non_goals.values()]:
        assert entry["status"] in {"open_policy_gap", "non_goal"}
        assert entry["counts_as_closed_parity"] is False
        _assert_evidence(entry["evidence"])


def test_future_compatibility_break_candidates_require_new_decisions() -> None:
    record = _load_record()
    candidates = {
        candidate["id"]: candidate
        for candidate in record["future_compatibility_break_candidates"]
    }

    assert {
        "remove_deprecated_event_routes",
    } <= set(candidates)

    for candidate in candidates.values():
        assert candidate["status"] == "future_decision_candidate"
        assert candidate["counts_as_closed_parity"] is False
        assert candidate["requires_new_contract_decision"] is True
        assert candidate["affected_surfaces"]
        _assert_evidence(candidate["evidence"])


def test_approved_policy_decisions_are_counted_as_closed_parity() -> None:
    record = _load_record()
    decisions = {
        decision["id"]: decision for decision in record["approved_policy_decisions"]
    }

    decision = decisions["trigger_repository_mutation_policy"]
    assert decision["status"] == "approved_current_behavior"
    assert decision["counts_as_closed_parity"] is True
    assert "project_path" in decision["current_boundary"]
    _assert_evidence(decision["evidence"])

    manager_authoring = decisions["manager_loop_authoring_surface_completeness"]
    assert manager_authoring["status"] == "approved_current_behavior"
    assert manager_authoring["counts_as_closed_parity"] is True
    assert "manager.steer_cooldown" in manager_authoring["current_boundary"]
    assert "stack.child_autostart" in manager_authoring["current_boundary"]
    _assert_evidence(manager_authoring["evidence"])

    manager_telemetry = decisions["manager_loop_telemetry_ingestion"]
    assert manager_telemetry["status"] == "approved_current_behavior"
    assert manager_telemetry["counts_as_closed_parity"] is True
    assert "context.stack.child.*" in manager_telemetry["current_boundary"]
    assert "failure context" in manager_telemetry["current_boundary"]
    _assert_evidence(manager_telemetry["evidence"])

    outgoing_edge_rule = decisions["non_exit_outgoing_edge_rule"]
    assert outgoing_edge_rule["status"] == "approved_current_behavior"
    assert outgoing_edge_rule["counts_as_closed_parity"] is True
    assert "node_has_outgoing_edge" in outgoing_edge_rule["current_boundary"]
    _assert_evidence(outgoing_edge_rule["evidence"])


def _load_record() -> dict[str, Any]:
    return json.loads(RECORD_PATH.read_text(encoding="utf-8"))


def _assert_evidence(evidence: list[dict[str, Any]]) -> None:
    assert evidence
    for reference in evidence:
        kind = reference["kind"]
        if kind == "fixture":
            fixture_id = reference["fixture_id"]
            assert (COMPAT_FIXTURE_ROOT / f"{fixture_id}.json").is_file(), fixture_id
            continue

        path = reference["path"]
        assert (REPO_ROOT / path).exists(), path
