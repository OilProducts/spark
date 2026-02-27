import json
from pathlib import Path

from attractor.release_gate_checks import validate_artifact_and_status_contract


def test_validate_artifact_and_status_contract_accepts_minimal_required_status_payload(tmp_path: Path) -> None:
    logs_root = tmp_path / "logs"
    (logs_root / "artifacts").mkdir(parents=True, exist_ok=True)
    stage_dir = logs_root / "plan"
    stage_dir.mkdir(parents=True, exist_ok=True)
    (stage_dir / "status.json").write_text(json.dumps({"outcome": "success"}), encoding="utf-8")

    errors = validate_artifact_and_status_contract(logs_root=logs_root, status_node_ids=("plan",))

    assert errors == []


def test_validate_artifact_and_status_contract_rejects_invalid_optional_field_type(tmp_path: Path) -> None:
    logs_root = tmp_path / "logs"
    (logs_root / "artifacts").mkdir(parents=True, exist_ok=True)
    stage_dir = logs_root / "plan"
    stage_dir.mkdir(parents=True, exist_ok=True)
    (stage_dir / "status.json").write_text(
        json.dumps({"outcome": "success", "notes": 123}),
        encoding="utf-8",
    )

    errors = validate_artifact_and_status_contract(logs_root=logs_root, status_node_ids=("plan",))

    assert "notes for node 'plan' must be a string" in errors
