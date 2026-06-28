from __future__ import annotations

import ast
import json
from pathlib import Path
import re


REPO_ROOT = Path(__file__).resolve().parents[2]
MIGRATION_DOC = REPO_ROOT / "docs" / "rust-rewrite-migration.md"
CONTRACT_DECISIONS = (
    REPO_ROOT / "specs" / "unified-llm-rust-runtime" / "contract-decisions.json"
)
RUST_LAUNCHER = REPO_ROOT / "src" / "spark" / "_rust_launcher.py"
BOUND_UNIFIED_LLM_DECISIONS = {
    "CD-ULLM-RUST-001",
    "CD-ULLM-RUST-015",
    "CD-ULLM-RUST-016",
    "CD-ULLM-RUST-018",
}
REQUIRED_MIGRATION_SECTIONS = {
    "Binding Unified LLM Contract Decisions",
    "Unified LLM Boundary",
    "LLM Runtime Inputs",
    "Deferred Unified LLM Capabilities",
    "Deprecated Compatibility Surfaces",
}
DECISION_BOUNDARY_CODE_VALUES = {
    "CD-ULLM-RUST-001": {
        "crates/spark-server",
        "crates/spark-cli",
        "crates/attractor-runtime",
        "crates/spark-agent-adapter",
        "crates/unified-llm-adapter",
        "src.unified_llm.adapters",
        "src.unified_llm.provider_utils",
    },
    "CD-ULLM-RUST-015": {
        "tests/compat/providers",
        "tests/adapters",
        "uv run pytest -q",
    },
    "CD-ULLM-RUST-016": {
        "crates/unified-llm-adapter",
        "src/unified_llm",
    },
    "CD-ULLM-RUST-018": {
        "crates/unified-llm-adapter/src/profiles.rs",
        "crates/unified-llm-adapter/src/resolution.rs",
        "crates/attractor-api/src/lib.rs",
        "llm-profiles.toml",
        "_attractor.runtime.launch_model",
        "_attractor.runtime.launch_provider",
        "_attractor.runtime.launch_profile",
        "_attractor.runtime.launch_reasoning_effort",
        "src/spark/llm_profiles.py",
    },
}
LLM_RUNTIME_INPUT_CODES = {
    "profile": {
        "llm-profiles.toml",
        "crates/unified-llm-adapter/src/profiles.rs",
        "id",
        "label",
        "provider",
        "models",
        "default_model",
        "configured",
        "base_url",
        "api_key_env",
        "src/spark/llm_profiles.py",
        "crates/unified-llm-adapter/tests/llm_profile_contracts.rs",
        "tests/test_llm_profiles.py",
        "tests/api/test_llm_profiles_endpoint.py",
    },
    "launch": {
        "_attractor.runtime.launch_model",
        "_attractor.runtime.launch_provider",
        "_attractor.runtime.launch_profile",
        "_attractor.runtime.launch_reasoning_effort",
        "crates/attractor-api/src/lib.rs",
        "crates/unified-llm-adapter/src/resolution.rs",
        "crates/attractor-api/tests/pipeline_lifecycle_contracts.rs",
        "crates/unified-llm-adapter/tests/runtime_boundary_contracts.rs",
        "crates/unified-llm-adapter/tests/public_surface_contracts.rs",
    },
}


def test_committed_migration_doc_uses_committed_sections_and_no_workflow_ledgers() -> None:
    document = MIGRATION_DOC.read_text(encoding="utf-8")

    assert REQUIRED_MIGRATION_SECTIONS <= set(_second_level_headings(document))
    for forbidden_text in (
        _workflow_current_path("rust" + "-rewrite", "migration-records.json"),
        _workflow_current_path("spec" + "-implementation"),
        "generated workflow ledger",
        "generated workflow ledgers",
        "workflow ledger",
        "workflow ledgers",
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

    for decision_id, required_codes in DECISION_BOUNDARY_CODE_VALUES.items():
        assert required_codes <= _code_values(documented_boundaries[decision_id])

    for decision_id in BOUND_UNIFIED_LLM_DECISIONS:
        decision = committed_decisions[decision_id]
        assert decision["requirement_ids"]
        assert decision["behavioral_contract"]
        assert decision["implementation_contract"]
        assert decision["validation_expectations"]


def test_committed_migration_doc_records_unified_llm_runtime_ownership() -> None:
    rows = _table_rows("Unified LLM Boundary")

    classifications = {_code_value(row["Classification"]) for row in rows}
    assert {"native_rust", "rust_owned_adapter", "retained_python_module"} <= classifications

    request_boundary = _row_containing(rows, "Surface", "Request DTOs")
    assert request_boundary["Current owner"] == "`crates/unified-llm-adapter`"
    assert request_boundary["Classification"] == "`native_rust`"

    routing_boundary = _row_containing(rows, "Surface", "Provider adapter routing")
    assert routing_boundary["Current owner"] == "`crates/unified-llm-adapter`"
    assert routing_boundary["Classification"] == "`rust_owned_adapter`"


def test_committed_migration_doc_keeps_python_unified_llm_as_compatibility_surface() -> None:
    rows = _table_rows("Unified LLM Boundary")

    retained = _row_containing(rows, "Surface", "Provider-specific clients")
    assert retained["Classification"] == "`retained_python_module`"
    assert retained["Current owner"] == "`src/unified_llm`"
    assert {
        "tests/adapters/test_openai_adapter.py",
        "tests/adapters/test_anthropic_adapter.py",
        "tests/adapters/test_cross_provider_parity.py",
    } <= _row_code_values(retained)

    launcher_imports = _python_import_modules(RUST_LAUNCHER)
    assert not any(
        module == "unified_llm" or module.startswith("unified_llm.")
        for module in launcher_imports
    )


def test_committed_migration_doc_records_llm_profiles_and_launch_context_inputs() -> None:
    rows = _table_rows("LLM Runtime Inputs")
    classifications = {_code_value(row["Classification"]) for row in rows}

    assert {"rust_owned_runtime_input"} <= classifications

    profile = _row_containing(rows, "Input", "llm-profiles.toml")
    assert profile["Classification"] == "`rust_owned_runtime_input`"
    assert LLM_RUNTIME_INPUT_CODES["profile"] <= _row_code_values(profile)

    launch = _row_containing(rows, "Input", "_attractor.runtime.launch_model")
    assert launch["Classification"] == "`rust_owned_runtime_input`"
    assert LLM_RUNTIME_INPUT_CODES["launch"] <= _row_code_values(launch)

    profile_dispatch = _row_containing(rows, "Input", "Profile-backed OpenAI-compatible")
    assert profile_dispatch["Classification"] == "`rust_owned_runtime_input`"
    assert {
        "openai_compatible",
        "require_api_key = false",
        "api_key_env",
        "crates/spark-agent-adapter/tests/llm_backend_contracts.rs",
        "crates/attractor-runtime/tests/core_handler_contracts.rs",
    } <= _row_code_values(profile_dispatch)

    decision = _contract_decisions()["CD-ULLM-RUST-018"]
    decision_text = json.dumps(decision)
    assert {
        "repository:src/spark/llm_profiles.py",
        "repository:crates/unified-llm-adapter/src/profiles.rs",
        "repository:crates/unified-llm-adapter/src/resolution.rs",
    } <= set(decision["spec_refs"])
    assert all(
        code in decision_text
        for code in LLM_RUNTIME_INPUT_CODES["launch"]
        if code.startswith("_attractor.")
    )


def test_committed_migration_doc_records_deferred_unified_llm_capabilities() -> None:
    rows = _table_rows("Deferred Unified LLM Capabilities")
    statuses = {_code_value(row["Status"]) for row in rows}
    evidence_codes = set().union(*(_row_code_values(row) for row in rows))

    assert {
        "non_goal_for_ordinary_validation",
        "future_work",
        "currently_rejected",
    } <= statuses
    assert {
        "CD-ULLM-RUST-015",
        "CD-ULLM-RUST-016",
        "CD-ULLM-RUST-017",
        "crates/unified-llm-adapter/tests/openai_compatible_contracts.rs",
        "crates/unified-llm-adapter/tests/native_request_contracts.rs",
    } <= evidence_codes


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


def test_committed_migration_doc_records_future_decisions_as_policy_boundaries() -> None:
    headings = set(_second_level_headings(MIGRATION_DOC.read_text(encoding="utf-8")))
    approved_rows = _list_items("Approved Policy Decisions")
    future_body = _section_body("Future Decision Candidates")

    assert {"Approved Policy Decisions", "Future Decision Candidates"} <= headings
    assert approved_rows
    assert future_body.strip()


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


def _row_code_values(row: dict[str, str]) -> set[str]:
    return set().union(*(_code_values(value) for value in row.values()))


def _code_values(value: str) -> set[str]:
    return set(re.findall(r"`([^`]+)`", value))


def _code_value(value: str) -> str:
    return value.strip("`")


def _second_level_headings(document: str) -> list[str]:
    return [
        line.removeprefix("## ").strip()
        for line in document.splitlines()
        if line.startswith("## ")
    ]


def _list_items(heading: str) -> list[str]:
    return [line for line in _section_body(heading).splitlines() if line.startswith("- ")]


def _section_body(heading: str) -> str:
    lines = MIGRATION_DOC.read_text(encoding="utf-8").splitlines()
    heading_line = f"## {heading}"
    start = lines.index(heading_line) + 1
    section_lines = []
    for line in lines[start:]:
        if line.startswith("## "):
            break
        section_lines.append(line)
    return "\n".join(section_lines)


def _workflow_current_path(workflow: str, *parts: str) -> str:
    return "/".join(("." + "spark", workflow, "cur" + "rent", *parts))


def _contract_decisions() -> dict[str, dict]:
    return {
        decision["id"]: decision
        for decision in json.loads(CONTRACT_DECISIONS.read_text(encoding="utf-8"))[
            "decisions"
        ]
    }


def _python_import_modules(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    modules: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            modules.add(node.module)
    return modules
