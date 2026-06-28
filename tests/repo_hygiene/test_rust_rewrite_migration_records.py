from __future__ import annotations

import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
MIGRATION_DOC = REPO_ROOT / "docs" / "rust-rewrite-migration.md"
CONTRACT_DECISIONS = (
    REPO_ROOT / "specs" / "unified-llm-rust-runtime" / "contract-decisions.json"
)
BOUND_UNIFIED_LLM_DECISIONS = {
    "CD-ULLM-RUST-001",
    "CD-ULLM-RUST-015",
    "CD-ULLM-RUST-016",
}


def test_committed_migration_doc_is_self_contained_source_of_truth() -> None:
    document = MIGRATION_DOC.read_text(encoding="utf-8")

    assert "This committed document is the migration source of truth" in document
    assert "Validation of this document uses committed repository files only" in document
    for forbidden_text in (
        ".spark/rust-rewrite/current/migration-records.json",
        ".spark/spec-implementation/current",
        "generated workflow ledger",
        "generated workflow ledgers",
        "workflow ledger",
        "workflow ledgers",
        "structured source of truth",
        "structured record",
    ):
        assert forbidden_text not in document


def test_committed_migration_doc_binds_unified_llm_contract_decisions() -> None:
    committed_decisions = _contract_decisions()
    rows = _table_rows("Binding Unified LLM Contract Decisions")
    documented_boundaries = {
        _code_value(row["Decision"]): row["Documentation boundary"] for row in rows
    }

    assert BOUND_UNIFIED_LLM_DECISIONS <= set(committed_decisions)
    assert BOUND_UNIFIED_LLM_DECISIONS <= set(documented_boundaries)

    runtime_boundary = documented_boundaries["CD-ULLM-RUST-001"]
    assert "Normal Spark server and CLI unified LLM execution is Rust-owned" in runtime_boundary
    assert "does not import, shell out to, or wrap `src.unified_llm` provider clients" in runtime_boundary

    validation_boundary = documented_boundaries["CD-ULLM-RUST-015"]
    assert "retained oracle or compatibility support" in validation_boundary
    assert "do not replace Rust-owned behavioral validation" in validation_boundary

    retained_surface_boundary = documented_boundaries["CD-ULLM-RUST-016"]
    assert "Rust-owned surfaces are" in retained_surface_boundary
    assert "Retained Python surfaces are" in retained_surface_boundary
    assert "Unsupported/deferred provider capabilities are" in retained_surface_boundary
    assert "Optional live smoke prerequisites are" in retained_surface_boundary


def test_committed_migration_doc_records_unified_llm_runtime_ownership() -> None:
    rows = _table_rows("Unified LLM Boundary")

    classifications = {_code_value(row["Classification"]) for row in rows}
    assert {"native_rust", "rust_owned_adapter", "retained_python_module"} <= classifications

    request_boundary = _row_containing(rows, "Surface", "Request DTOs")
    assert request_boundary["Current owner"] == "`crates/unified-llm-adapter`"
    assert request_boundary["Classification"] == "`native_rust`"
    assert "normal Spark server and CLI contract" in request_boundary["Retained Python status"]

    routing_boundary = _row_containing(rows, "Surface", "Provider adapter routing")
    assert routing_boundary["Current owner"] == "`crates/unified-llm-adapter`"
    assert routing_boundary["Classification"] == "`rust_owned_adapter`"
    assert "not wrappers used by Spark server or CLI execution" in routing_boundary[
        "Retained Python status"
    ]


def test_committed_migration_doc_keeps_python_unified_llm_as_compatibility_surface() -> None:
    rows = _table_rows("Unified LLM Boundary")

    retained = _row_containing(rows, "Surface", "Provider-specific clients")
    assert retained["Classification"] == "`retained_python_module`"
    assert retained["Current owner"] == "`src/unified_llm`"
    assert "compatibility, oracle, and package-data support" in retained[
        "Retained Python status"
    ]

    document = MIGRATION_DOC.read_text(encoding="utf-8")
    assert "Retained Python checks remain `uv run pytest tests/compat/providers -q`" in document
    assert "they do not replace the Rust-owned runtime validation" in document


def test_committed_migration_doc_records_deprecated_surfaces_as_contract_boundaries() -> None:
    rows = _table_rows("Deprecated Compatibility Surfaces")

    by_surface = {row["Surface"]: row for row in rows}
    assert by_surface["`GET /attractor/runs/events`"]["Current status"] == (
        "Preserved compatibility route"
    )
    assert by_surface["`GET /workspace/api/conversations/{conversation_id}/events`"][
        "Preferred replacement"
    ] == "`GET /workspace/api/live/events`"
    assert by_surface["`GET /attractor/pipelines/{id}/events`"]["Evidence"]


def test_committed_migration_doc_records_non_goals_and_future_decisions() -> None:
    document = MIGRATION_DOC.read_text(encoding="utf-8")

    assert "Credential-backed smoke execution in ordinary validation is an explicit non-goal" in document
    assert "M7 does not remove Python `agent` or `unified_llm` modules" in document
    assert "Any removal of deprecated event routes" in document
    assert "requires a new contract decision before implementation depends on it" in document


def _table_rows(heading: str) -> list[dict[str, str]]:
    lines = MIGRATION_DOC.read_text(encoding="utf-8").splitlines()
    heading_line = f"## {heading}"
    start = lines.index(heading_line) + 1
    table_lines = []
    for line in lines[start:]:
        if line.startswith("## ") and table_lines:
            break
        if line.startswith("|"):
            table_lines.append(line)
        elif table_lines:
            break

    assert len(table_lines) >= 3, heading
    headers = _split_row(table_lines[0])
    body = [
        _split_row(line)
        for line in table_lines[2:]
        if set(line.replace("|", "").strip()) != {"-"}
    ]
    return [dict(zip(headers, row, strict=True)) for row in body]


def _split_row(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def _row_containing(rows: list[dict[str, str]], column: str, text: str) -> dict[str, str]:
    for row in rows:
        if text in row[column]:
            return row
    raise AssertionError(f"missing row containing {text!r}")


def _code_value(value: str) -> str:
    return value.strip("`")


def _contract_decisions() -> dict[str, dict]:
    return {
        decision["id"]: decision
        for decision in json.loads(CONTRACT_DECISIONS.read_text(encoding="utf-8"))[
            "decisions"
        ]
    }
