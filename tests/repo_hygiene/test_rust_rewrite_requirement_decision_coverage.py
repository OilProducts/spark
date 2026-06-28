from __future__ import annotations

import ast
import json
from pathlib import Path
from pathlib import PurePosixPath
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
    f"{'/'.join(('.' + 'spark', 'rust' + '-rewrite', 'cur' + 'rent'))}/",
    f"{'/'.join(('.' + 'spark', 'spec' + '-implementation', 'cur' + 'rent'))}/",
)
WORKFLOW_CURRENT_PATH_COMPONENTS = frozenset(
    {
        "." + "spark",
        "rust" + "-rewrite",
        "spec" + "-implementation",
        "cur" + "rent",
    }
)
WORKFLOW_CURRENT_ALIAS_TEST_ALLOWLIST = {
    "tests/repo_hygiene/test_spec_implementation_flow_contracts.py": {
        "allowed_aliases": (WORKFLOW_CURRENT_ALIASES[1],),
        "reason": (
            "Product contract coverage for the spec-implementation workflow; it "
            "asserts the runtime alias paths that workflow exposes and executes."
        ),
    },
}
PATH_COMPONENT_LITERAL_RE = re.compile(
    r"(?P<quote>['\"])(?P<component>"
    + "|".join(re.escape(component) for component in sorted(WORKFLOW_CURRENT_PATH_COMPONENTS))
    + r")(?P=quote)"
)
PATH_COMPONENT_SCAN_WINDOW_LINES = 8


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


def test_tracked_validation_tests_do_not_use_workflow_current_aliases_as_inputs() -> None:
    violations = [
        violation
        for tracked_path in _tracked_validation_test_paths()
        for violation in _workflow_current_reference_violations(
            tracked_path,
            (REPO_ROOT / tracked_path).read_text(encoding="utf-8"),
            allowed_aliases=_allowed_workflow_current_aliases(tracked_path),
        )
    ]

    assert violations == []


def test_workflow_current_alias_allowlist_is_limited_to_spec_workflow_contracts() -> None:
    tracked_paths = set(_git_ls_files())

    assert set(WORKFLOW_CURRENT_ALIAS_TEST_ALLOWLIST) <= tracked_paths
    assert all(
        "spec-implementation workflow" in entry["reason"]
        for entry in WORKFLOW_CURRENT_ALIAS_TEST_ALLOWLIST.values()
    )
    assert all(
        set(entry["allowed_aliases"]) == {WORKFLOW_CURRENT_ALIASES[1]}
        for entry in WORKFLOW_CURRENT_ALIAS_TEST_ALLOWLIST.values()
    )


def test_workflow_current_reference_violations_report_path_and_line() -> None:
    tracked_path = "crates/example/tests/example_contracts.rs"
    source = "\n".join(
        [
            "fn fixture_path() -> &'static str {",
            f'    "{WORKFLOW_CURRENT_ALIASES[0]}compat-fixtures/providers"',
            "}",
        ]
    )

    assert _workflow_current_reference_violations(tracked_path, source) == [
        f"{tracked_path}:2: workflow current alias reference: {source.splitlines()[1].strip()}"
    ]


def test_assembled_workflow_current_path_references_report_path_and_line() -> None:
    tracked_path = "crates/example/tests/example_contracts.rs"
    spark_component = next(
        component
        for component in WORKFLOW_CURRENT_PATH_COMPONENTS
        if component.startswith(".")
    )
    rust_workflow_component = next(
        component
        for component in WORKFLOW_CURRENT_PATH_COMPONENTS
        if component.startswith("rust")
    )
    current_component = next(
        component
        for component in WORKFLOW_CURRENT_PATH_COMPONENTS
        if component.startswith("cur")
    )
    source = "\n".join(
        [
            "let path = root",
            f'    .join("{spark_component}")',
            f'    .join("{rust_workflow_component}")',
            f'    .join("{current_component}")',
            '    .join("compat-fixtures");',
        ]
    )

    assert _workflow_current_reference_violations(tracked_path, source) == [
        (
            f"{tracked_path}:2: assembled workflow current alias path components: "
            f"{source.splitlines()[1].strip()}"
        )
    ]


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


def _tracked_validation_test_paths() -> list[str]:
    return [
        path
        for path in _git_ls_files()
        if _is_tracked_validation_test_path(path)
    ]


def _is_tracked_validation_test_path(path: str) -> bool:
    posix_path = PurePosixPath(path)
    parts = posix_path.parts

    if (
        len(parts) >= 4
        and parts[0] == "crates"
        and parts[2] == "tests"
        and posix_path.suffix == ".rs"
    ):
        return True

    return (
        len(parts) >= 2
        and parts[0] == "tests"
        and posix_path.suffix == ".py"
        and posix_path.name.startswith("test_")
    )


def _workflow_current_reference_violations(
    tracked_path: str,
    source: str,
    allowed_aliases: Iterable[str] = (),
) -> list[str]:
    lines = source.splitlines()
    violations: list[str] = []
    violation_lines: set[int] = set()
    allowed_alias_prefixes = {alias.rstrip("/") for alias in allowed_aliases}
    allowed_workflow_components = {
        component
        for alias in allowed_aliases
        for component in ("rust" + "-rewrite", "spec" + "-implementation")
        if component in alias
    }

    for line_number, line in enumerate(lines, start=1):
        if any(
            alias.rstrip("/") in line and alias.rstrip("/") not in allowed_alias_prefixes
            for alias in WORKFLOW_CURRENT_ALIASES
        ):
            violations.append(
                _format_workflow_current_violation(
                    tracked_path,
                    line_number,
                    line,
                    "workflow current alias reference",
                )
            )
            violation_lines.add(line_number)

    component_sets_by_line = [_path_components_in_line(line) for line in lines]
    for line_index, components in enumerate(component_sets_by_line):
        if "." + "spark" not in components:
            continue
        window_components = set().union(
            *component_sets_by_line[
                line_index : line_index + PATH_COMPONENT_SCAN_WINDOW_LINES
            ]
        )
        workflow_components = {
            "rust" + "-rewrite",
            "spec" + "-implementation",
        } & window_components
        if (
            "cur" + "rent" in window_components
            and workflow_components - allowed_workflow_components
        ):
            line_number = line_index + 1
            if line_number not in violation_lines:
                violations.append(
                    _format_workflow_current_violation(
                        tracked_path,
                        line_number,
                        lines[line_index],
                        "assembled workflow current alias path components",
                    )
                )

    return violations


def _allowed_workflow_current_aliases(tracked_path: str) -> Iterable[str]:
    entry = WORKFLOW_CURRENT_ALIAS_TEST_ALLOWLIST.get(tracked_path)
    if entry is None:
        return ()
    return entry["allowed_aliases"]


def _path_components_in_line(line: str) -> set[str]:
    return {
        match.group("component")
        for match in PATH_COMPONENT_LITERAL_RE.finditer(line)
    }


def _format_workflow_current_violation(
    tracked_path: str,
    line_number: int,
    line: str,
    reason: str,
) -> str:
    return f"{tracked_path}:{line_number}: {reason}: {line.strip()}"


def _git_ls_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]
