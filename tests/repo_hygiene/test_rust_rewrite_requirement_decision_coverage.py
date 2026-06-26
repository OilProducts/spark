from __future__ import annotations

import json
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
CURRENT = REPO_ROOT / ".spark" / "rust-rewrite" / "current"
STATE_PATH = CURRENT / "state.json"
REQUIREMENTS_PATH = CURRENT / "requirements.json"
DECISIONS_PATH = CURRENT / "contract-decisions.json"
ARTIFACT_PATH = CURRENT / "validation" / "requirement-decision-coverage-review.json"

ITEM_ID = "M7-I05-REQUIREMENT-DECISION-COVERAGE-REVIEW"
MILESTONE_ID = "M7-DOCS-VALIDATION"
BOUND_REQUIREMENTS = {
    "RR-DOC-001",
    "RR-DOC-002",
    "RR-DOC-003",
    "RR-VAL-001",
    "RR-VAL-002",
    "RR-VAL-003",
    "RR-VAL-004",
    "RR-VAL-005",
}
BOUND_DECISIONS = {"CD-RR-001", "CD-RR-013", "CD-RR-014", "CD-RR-015"}
SPLIT_GATES = {
    "RR-DSL-006__RR-STO-004",
    "RR-STO-005__RR-STO-003",
    "RR-EXE-006__RR-PKG-005",
    "RR-EXE-008__RR-WKF-005",
    "RR-WKF-005__RR-WKF-004",
    "RR-API-004__RR-WKF-004",
    "RR-API-001__RR-WKF-004",
    "RR-API-003__RR-PKG-002",
    "RR-STO-004__RR-PKG-002",
}
ALLOWED_STATUSES = {
    "pass",
    "boundary_documented",
    "prerequisite_limited",
    "pending_ledger_status",
    "missing_evidence",
    "fail",
}


def test_coverage_review_is_bound_to_active_item_contracts() -> None:
    artifact = _load_artifact()

    assert artifact["schema_version"] == "rust-rewrite-requirement-decision-coverage-review-v1"
    assert artifact["item_id"] == ITEM_ID
    assert artifact["milestone_id"] == MILESTONE_ID
    assert set(artifact["requirements"]) == BOUND_REQUIREMENTS
    assert set(artifact["decisions"]) == BOUND_DECISIONS
    assert Path(artifact["worktree_path"]).resolve() == REPO_ROOT
    assert artifact["status"] in ALLOWED_STATUSES


def test_every_requirement_has_evidence_or_an_explicit_non_passing_note() -> None:
    artifact = _load_artifact()
    requirements = _load_json(REQUIREMENTS_PATH)["requirements"]
    coverage = artifact["requirement_coverage"]

    assert set(coverage) == {requirement["id"] for requirement in requirements}

    pending_ids = set()
    for requirement in requirements:
        entry = coverage[requirement["id"]]
        assert entry["ledger_status"] == requirement["status"]
        assert entry["status"] in ALLOWED_STATUSES
        assert isinstance(entry["counts_as_passing"], bool)
        _assert_evidence_or_note(entry)
        if entry["status"] == "pending_ledger_status":
            pending_ids.add(requirement["id"])
            assert entry["counts_as_passing"] is False

    assert pending_ids == set()


def test_every_contract_decision_has_review_evidence() -> None:
    artifact = _load_artifact()
    decisions = _load_json(DECISIONS_PATH)["decisions"]
    coverage = artifact["decision_coverage"]

    assert set(coverage) == {decision["id"] for decision in decisions}

    for decision in decisions:
        entry = coverage[decision["id"]]
        assert entry["status"] in ALLOWED_STATUSES
        assert isinstance(entry["counts_as_passing"], bool)
        assert entry["related_requirements"] == decision["requirement_ids"]
        _assert_evidence_or_note(entry)


def test_architecture_split_gates_are_represented_with_evidence_or_notes() -> None:
    artifact = _load_artifact()
    split_gates = {entry["id"]: entry for entry in artifact["split_gate_coverage"]}

    assert set(split_gates) == SPLIT_GATES

    for entry in split_gates.values():
        assert len(entry["requirements"]) >= 2
        assert entry["status"] in ALLOWED_STATUSES
        assert isinstance(entry["counts_as_passing"], bool)
        if entry["status"] == "pending_ledger_status":
            assert entry["counts_as_passing"] is False
            assert entry["notes"]
        _assert_evidence_or_note(
            {
                "evidence": entry["observed_evidence"],
                "notes": entry["notes"],
            }
        )


def test_referenced_evidence_paths_are_scoped_to_the_rewrite_worktree() -> None:
    artifact = _load_artifact()

    for path_value in artifact["generated_evidence_paths"].values():
        _assert_rewrite_path(path_value)

    for entry in artifact["requirement_coverage"].values():
        _assert_evidence_paths(entry["evidence"])

    for entry in artifact["decision_coverage"].values():
        _assert_evidence_paths(entry["evidence"])

    for entry in artifact["split_gate_coverage"]:
        _assert_evidence_paths(entry["observed_evidence"])

    for note in artifact["retained_boundary_notes"]:
        _assert_evidence_paths(note["evidence"])


def test_prerequisite_limited_and_pending_entries_do_not_count_as_passing() -> None:
    artifact = _load_artifact()

    pending = artifact["pending_ledger_notes"]
    limited = artifact["prerequisite_limited_notes"]

    assert pending == []
    assert limited == []

    for entry in artifact["requirement_coverage"].values():
        if entry["status"] in {"pending_ledger_status", "prerequisite_limited", "missing_evidence", "fail"}:
            assert entry["counts_as_passing"] is False

    for note in [*artifact["retained_boundary_notes"], *limited]:
        if note["status"] in {"prerequisite_limited", "open_policy_gap", "non_goal", "future_decision_candidate"}:
            assert note["counts_as_passing"] is False


def test_acceptance_workflow_harness_coverage_is_closed_when_final_validation_passes() -> None:
    artifact = _load_artifact()
    final_validation = _load_json(CURRENT / "validation" / "final-validation-artifacts.json")

    assert final_validation["acceptance_workflow_status"]["status"] == "pass"
    assert final_validation["acceptance_workflow_status"]["counts_as_passing"] is True

    rr_val_005 = artifact["requirement_coverage"]["RR-VAL-005"]
    assert rr_val_005["status"] == "pass"
    assert rr_val_005["counts_as_passing"] is True
    assert final_validation["acceptance_workflow_status"]["missing_prerequisites"] == []
    assert all(record["status"] != "prerequisite_limited" for record in rr_val_005["evidence"])

    closed = [
        note
        for note in artifact["retained_boundary_notes"]
        if note["id"] == "acceptance_workflow_harness"
        and note["classification"] == "closed_policy_gaps"
    ]
    assert closed
    assert all(note["counts_as_passing"] is True for note in closed)


def test_python_validation_command_evidence_uses_uv_pytest() -> None:
    artifact = _load_artifact()
    command_path = artifact["generated_evidence_paths"]["item_validation_commands"]
    commands = json.loads(Path(command_path).read_text(encoding="utf-8"))

    assert commands
    for command in commands:
        assert command.startswith("uv run pytest")


def test_prior_milestone_completion_uses_approved_evidence_and_records_stale_state() -> None:
    artifact = _load_artifact()
    prior_path = Path(artifact["generated_evidence_paths"]["prior_milestone_evidence"])
    prior = _load_json(prior_path)

    assert prior["status"] == "pass"
    assert prior["errors"] == []

    stale_notes = prior["stale_state_notes"]
    assert stale_notes

    for record in prior["milestones"]:
        assert record["completion_status"] == "completed"
        assert record["milestones_json_status"] == "completed"
        assert record["result_status"] == "completed"
        assert record["completed_item_count"] == record["item_count"]
        assert record["counts_as_passing"] is True
        if record["state_status"] != "completed":
            assert record["notes"]


def test_pass_review_and_result_do_not_reference_failed_generated_evidence() -> None:
    artifact = _load_artifact()

    failed_supporting = [
        status
        for status in artifact["supporting_artifact_statuses"]
        if status["status"] == "fail"
    ]
    failed_generated = []
    for path_value in artifact["generated_evidence_paths"].values():
        path = Path(path_value)
        if path.suffix != ".json":
            continue
        payload = _load_json(path)
        if not isinstance(payload, dict):
            continue
        if payload.get("status") == "fail":
            failed_generated.append(str(path))

    if artifact["status"] == "pass":
        assert failed_supporting == []
        assert failed_generated == []

    result = _load_json(CURRENT / "validation-result.json")
    if result["status"] == "pass":
        assert result["coverage_review_status"] == "pass"
        assert [
            status
            for status in result["supporting_artifact_statuses"]
            if status["status"] == "fail"
        ] == []


def _load_artifact() -> dict[str, Any]:
    return _load_json(ARTIFACT_PATH)


def _load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _assert_evidence_or_note(entry: dict[str, Any]) -> None:
    if entry.get("evidence"):
        _assert_evidence_paths(entry["evidence"])
        return
    assert entry.get("notes")
    assert entry.get("status") != "pass"


def _assert_evidence_paths(evidence: list[dict[str, Any]]) -> None:
    for record in evidence:
        path_value = record.get("path")
        if path_value:
            _assert_rewrite_path(path_value)


def _assert_rewrite_path(path_value: str) -> None:
    path = Path(path_value)
    assert path.is_absolute(), path_value
    assert path.exists(), path_value
    path.resolve().relative_to(REPO_ROOT)

    source_repo = Path(_load_json(STATE_PATH)["source_repo_path"]).resolve()
    if source_repo != REPO_ROOT:
        try:
            path.resolve().relative_to(source_repo)
        except ValueError:
            pass
        else:
            raise AssertionError(f"evidence path points at source repo: {path}")
