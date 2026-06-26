from __future__ import annotations

import json
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
STATE_PATH = REPO_ROOT / ".spark" / "rust-rewrite" / "current" / "state.json"
ARTIFACT_PATH = (
    REPO_ROOT
    / ".spark"
    / "rust-rewrite"
    / "current"
    / "validation"
    / "final-validation-artifacts.json"
)

ITEM_ID = "M7-I04-FINAL-VALIDATION-ARTIFACTS"
MILESTONE_ID = "M7-DOCS-VALIDATION"
BOUND_REQUIREMENTS = {
    "RR-VAL-001",
    "RR-VAL-002",
    "RR-VAL-003",
    "RR-VAL-004",
    "RR-VAL-005",
}
BOUND_DECISIONS = {"CD-RR-001", "CD-RR-013", "CD-RR-015"}
ACTIVE_COMMANDS = {
    "cargo fmt --check",
    "cargo test --workspace --all-features",
    "uv run pytest -q",
    "npm --prefix frontend run test:unit",
    "npm --prefix frontend run build",
    "npm --prefix frontend run ui:smoke",
    "just deliverable",
}
FINAL_STATUSES = {"pass", "fail", "prerequisite_limited"}
IN_PROGRESS_STATUSES = FINAL_STATUSES | {"in_progress", "not_run"}


def test_final_validation_artifact_is_bound_to_active_item_contracts() -> None:
    artifact = _load_artifact()

    assert artifact["schema_version"] == "rust-rewrite-final-validation-artifacts-v1"
    assert artifact["item_id"] == ITEM_ID
    assert artifact["milestone_id"] == MILESTONE_ID
    assert set(artifact["requirements"]) == BOUND_REQUIREMENTS
    assert set(artifact["decisions"]) == BOUND_DECISIONS
    assert Path(artifact["worktree_path"]).resolve() == REPO_ROOT


def test_active_validation_commands_have_structured_records_and_evidence() -> None:
    artifact = _load_artifact()
    records = {record["command"]: record for record in artifact["command_records"]}

    assert set(records) == ACTIVE_COMMANDS
    assert len(artifact["command_records"]) == len(ACTIVE_COMMANDS)

    allowed_statuses = (
        IN_PROGRESS_STATUSES
        if artifact["status"] == "in_progress"
        else FINAL_STATUSES
    )
    for command, record in records.items():
        assert record["domain"]
        assert record["status"] in allowed_statuses
        assert record["summary"]
        assert record["first_actionable_triage"]
        assert record["evidence_paths"], command
        for evidence_path in record["evidence_paths"]:
            _assert_path_is_rewrite_evidence(evidence_path)

        if record["status"] == "pass":
            assert record["exit_code"] == 0
            assert record["missing_prerequisites"] == []
        elif record["status"] == "prerequisite_limited":
            assert record["exit_code"] is None
            assert record["missing_prerequisites"]
        elif record["status"] == "fail":
            assert isinstance(record["exit_code"], int)
            assert record["exit_code"] != 0


def test_python_validation_records_use_uv_pytest() -> None:
    artifact = _load_artifact()

    python_records = [
        record
        for record in artifact["command_records"]
        if record["domain"] == "python"
    ]

    assert python_records
    for record in python_records:
        assert record["command"].startswith("uv run pytest")


def test_validation_domains_cover_frontend_packaging_acceptance_and_hygiene() -> None:
    artifact = _load_artifact()
    coverage = artifact["validation_domain_coverage"]

    assert {
        "rust_tests",
        "python_pytest",
        "frontend_unit",
        "frontend_build",
        "frontend_smoke",
        "packaging_deliverable",
        "acceptance_workflows",
        "artifact_hygiene",
    } <= set(coverage)

    for domain_name, domain in coverage.items():
        assert domain["status"] in {
            "pass",
            "fail",
            "prerequisite_limited",
            "in_progress",
            "not_run",
            "missing",
        }, domain_name
        assert domain["requirements"], domain_name
        assert domain["evidence_paths"], domain_name
        for evidence_path in domain["evidence_paths"]:
            _assert_path_is_rewrite_evidence(evidence_path)


def test_prerequisite_limited_evidence_is_not_counted_as_passing() -> None:
    artifact = _load_artifact()

    limited_records = [
        record
        for record in artifact["command_records"]
        if record["status"] == "prerequisite_limited"
    ]
    for record in limited_records:
        assert record["status"] != "pass"
        assert record["missing_prerequisites"]

    acceptance = artifact["acceptance_workflow_status"]
    if acceptance["status"] == "prerequisite_limited":
        assert acceptance["counts_as_passing"] is False
        assert acceptance["missing_prerequisites"]


def _load_artifact() -> dict[str, Any]:
    return json.loads(ARTIFACT_PATH.read_text(encoding="utf-8"))


def _assert_path_is_rewrite_evidence(path_value: str) -> None:
    path = Path(path_value)
    assert path.is_absolute(), path_value
    assert path.exists(), path_value
    path.resolve().relative_to(REPO_ROOT)

    source_repo = Path(
        json.loads(STATE_PATH.read_text(encoding="utf-8"))["source_repo_path"]
    ).resolve()
    if source_repo != REPO_ROOT:
        try:
            path.resolve().relative_to(source_repo)
        except ValueError:
            pass
        else:
            raise AssertionError(f"evidence path points at source repo: {path}")
