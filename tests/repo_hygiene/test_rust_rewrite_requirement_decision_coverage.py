from __future__ import annotations

import ast
import json
from pathlib import Path
import re
import subprocess
import tomllib
from typing import Any, Iterable


REPO_ROOT = Path(__file__).resolve().parents[2]
RUST_RUNTIME_CRATE_PATHS = (
    "crates/spark-server",
    "crates/spark-cli",
    "crates/spark-http",
    "crates/attractor-runtime",
    "crates/spark-agent-adapter",
    "crates/unified-llm-adapter",
)
RUST_SOURCE_SUFFIXES = {".rs", ".toml"}
RUST_UNIFIED_LLM_VALIDATION_TESTS = {
    "crates/unified-llm-adapter/tests/adapter_contracts.rs",
    "crates/unified-llm-adapter/tests/compatible_provider_parity_matrix.rs",
    "crates/unified-llm-adapter/tests/http_transport_contracts.rs",
    "crates/unified-llm-adapter/tests/llm_profile_contracts.rs",
    "crates/unified-llm-adapter/tests/native_provider_parity_matrix.rs",
    "crates/unified-llm-adapter/tests/native_request_contracts.rs",
    "crates/unified-llm-adapter/tests/openai_compatible_contracts.rs",
    "crates/unified-llm-adapter/tests/public_surface_contracts.rs",
    "crates/unified-llm-adapter/tests/runtime_boundary_contracts.rs",
    "crates/spark-agent-adapter/tests/llm_backend_contracts.rs",
    "crates/attractor-runtime/tests/core_handler_contracts.rs",
    "crates/spark-cli/tests/cli_shell_contracts.rs",
    "crates/spark-server/tests/server_shell_contracts.rs",
}
RETAINED_PYTHON_ORACLE_TESTS = {
    "tests/adapters/test_anthropic_adapter.py",
    "tests/adapters/test_cross_provider_parity.py",
    "tests/adapters/test_gemini_adapter.py",
    "tests/adapters/test_openai_adapter.py",
    "tests/adapters/test_openai_compatible_adapter.py",
    "tests/compat/providers/test_unified_llm_adapter_fixtures.py",
    "tests/compat/providers/test_unified_llm_runtime_boundary.py",
}
COMMITTED_UNIFIED_LLM_MANIFESTS = (
    REPO_ROOT / "specs" / "unified-llm-rust-runtime" / "requirements.json",
    REPO_ROOT / "specs" / "unified-llm-rust-runtime" / "contract-decisions.json",
    REPO_ROOT / "specs" / "unified-llm-spec-md" / "requirements.json",
    REPO_ROOT / "specs" / "unified-llm-spec-md" / "contract-decisions.json",
)
PROHIBITED_PROVIDER_CLIENT_REFERENCE = re.compile(
    r"\b(?:src\.)?unified_llm\.(?:adapters|provider_utils)\b"
    r"|\bfrom\s+(?:src\.)?unified_llm\s+import\s+[^#\n]*\b(?:adapters|provider_utils)\b"
)
WORKFLOW_CURRENT_ALIASES = (
    ".spark/rust-rewrite/current/",
    ".spark/spec-implementation/current/",
)


def test_cd_ullm_rust_001_runtime_reaches_rust_adapter_through_manifests() -> None:
    workspace = _load_toml(REPO_ROOT / "Cargo.toml")
    assert set(RUST_RUNTIME_CRATE_PATHS) <= set(workspace["workspace"]["members"])

    manifests = {
        crate_path: _load_toml(REPO_ROOT / crate_path / "Cargo.toml")
        for crate_path in RUST_RUNTIME_CRATE_PATHS
    }

    assert _dependency_names(manifests["crates/spark-server"]) >= {
        "attractor-runtime",
        "spark-http",
        "unified-llm-adapter",
    }
    assert _dependency_names(manifests["crates/spark-http"]) >= {
        "unified-llm-adapter",
    }
    assert _dependency_names(manifests["crates/attractor-runtime"]) >= {
        "spark-agent-adapter",
        "unified-llm-adapter",
    }
    assert _dependency_names(manifests["crates/spark-agent-adapter"]) >= {
        "unified-llm-adapter",
    }


def test_cd_ullm_rust_001_public_commands_are_rust_launcher_entry_points() -> None:
    pyproject = _load_toml(REPO_ROOT / "pyproject.toml")

    assert pyproject["project"]["scripts"] == {
        "spark": "spark._rust_launcher:spark_main",
        "spark-server": "spark._rust_launcher:spark_server_main",
    }
    assert "bin/*" in pyproject["tool"]["setuptools"]["package-data"]["spark"]

    launcher_imports = _python_import_modules(
        REPO_ROOT / "src" / "spark" / "_rust_launcher.py"
    )
    assert not any(
        module == "unified_llm" or module.startswith("unified_llm.")
        for module in launcher_imports
    )


def test_cd_ullm_rust_001_rust_runtime_sources_do_not_import_python_provider_clients() -> None:
    violations = [
        f"{path.relative_to(REPO_ROOT)}:{line_number}: {line.strip()}"
        for path in _rust_runtime_source_files()
        for line_number, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(),
            start=1,
        )
        if PROHIBITED_PROVIDER_CLIENT_REFERENCE.search(line)
    ]

    assert violations == []


def test_cd_ullm_rust_015_validation_surfaces_are_committed_behavior_tests() -> None:
    tracked_paths = set(_git_ls_files())
    decision = _contract_decisions()["CD-ULLM-RUST-015"]

    assert set(decision["requirement_ids"]) == {"REQ-001", "REQ-023", "REQ-024"}
    assert RUST_UNIFIED_LLM_VALIDATION_TESTS <= tracked_paths
    assert RETAINED_PYTHON_ORACLE_TESTS <= tracked_paths


def test_committed_unified_llm_manifests_do_not_depend_on_workflow_current_aliases() -> None:
    manifest_strings = [
        value
        for manifest_path in COMMITTED_UNIFIED_LLM_MANIFESTS
        for value in _walk_json_strings(json.loads(manifest_path.read_text(encoding="utf-8")))
    ]

    assert [
        value
        for value in manifest_strings
        if any(alias in value for alias in WORKFLOW_CURRENT_ALIASES)
    ] == []


def _load_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _dependency_names(manifest: dict[str, Any]) -> set[str]:
    return set(manifest.get("dependencies", {}))


def _python_import_modules(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    modules: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            modules.add(node.module)
    return modules


def _rust_runtime_source_files() -> list[Path]:
    return sorted(
        path
        for crate_path in RUST_RUNTIME_CRATE_PATHS
        for path in (REPO_ROOT / crate_path).rglob("*")
        if path.is_file() and path.suffix in RUST_SOURCE_SUFFIXES
    )


def _contract_decisions() -> dict[str, dict[str, Any]]:
    payload = json.loads(
        (
            REPO_ROOT
            / "specs"
            / "unified-llm-rust-runtime"
            / "contract-decisions.json"
        ).read_text(encoding="utf-8")
    )
    return {decision["id"]: decision for decision in payload["decisions"]}


def _walk_json_strings(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from _walk_json_strings(item)
    elif isinstance(value, dict):
        for item in value.values():
            yield from _walk_json_strings(item)


def _git_ls_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]
