from __future__ import annotations

from collections import Counter, defaultdict
import json
from pathlib import Path
import subprocess
from typing import Any, Mapping

from tests.compat import harness


ITEM_ID = "M0-I06-COVERAGE-VALIDATION-GATE"
MILESTONE_ID = "M0-COMPAT-HARNESS"
REQUIREMENTS = {"RR-VAL-001", "RR-VAL-002"}
DECISIONS = {"CD-RR-001", "CD-RR-013", "CD-RR-015"}
KNOWN_FIXTURE_ITEM_IDS = {
    "M0-I02-CLI-FILESYSTEM-FIXTURES",
    "M0-I03-HTTP-SSE-FIXTURES",
    "M0-I04-DSL-RUNTIME-FIXTURES",
    "M0-I05-FRONTEND-PACKAGING-FIXTURES",
}
EXPECTED_GROUPS = {
    "cli",
    "filesystem",
    "http",
    "sse",
    "dsl",
    "runtime",
    "frontend",
    "packaging",
}
EXPECTED_DOMAIN_SLUGS = {
    "cli-filesystem",
    "http-sse",
    "dsl-runtime",
    "frontend-packaging",
    "python-guardrail",
    "triage",
    "hygiene",
    "closure-evidence",
}


def test_reviewed_fixture_manifests_have_known_m0_coverage(
    compat_fixture_root: Path,
) -> None:
    manifests = _m0_fixture_manifests(compat_fixture_root)

    assert manifests
    observed_groups: Counter[str] = Counter()
    item_groups: dict[str, set[str]] = defaultdict(set)
    for path, manifest in manifests:
        relative = path.relative_to(compat_fixture_root)
        group = relative.parts[0]
        observed_groups[group] += 1

        item_id = str(manifest.get("item_id", ""))
        assert item_id in KNOWN_FIXTURE_ITEM_IDS
        item_groups[item_id].add(group)

        coverage = harness.validate_manifest_coverage(
            manifest,
            requirement_ids=REQUIREMENTS,
            decision_ids=DECISIONS,
        )
        assert set(coverage["requirements"]) >= REQUIREMENTS
        assert set(coverage["decisions"]) >= DECISIONS
        assert str(manifest.get("fixture_id", "")).startswith(f"{group}/")

    assert set(observed_groups) == EXPECTED_GROUPS
    assert all(count > 0 for count in observed_groups.values())
    assert item_groups["M0-I02-CLI-FILESYSTEM-FIXTURES"] == {"cli", "filesystem"}
    assert item_groups["M0-I03-HTTP-SSE-FIXTURES"] == {"http", "sse"}
    assert item_groups["M0-I04-DSL-RUNTIME-FIXTURES"] == {"dsl", "runtime"}
    assert item_groups["M0-I05-FRONTEND-PACKAGING-FIXTURES"] == {
        "frontend",
        "packaging",
    }


def test_coverage_ledger_maps_fixture_groups_contracts_and_m0_acceptance(
    compat_fixture_root: Path,
    compat_validation_root: Path,
) -> None:
    ledger = _load_json(compat_validation_root / "m0-coverage-ledger.json")
    manifests = _m0_fixture_manifests(compat_fixture_root)
    observed_counts = Counter(path.relative_to(compat_fixture_root).parts[0] for path, _ in manifests)

    assert ledger["schema_version"] == "compat-m0-coverage-ledger-v1"
    assert ledger["milestone_id"] == MILESTONE_ID
    assert ledger["active_item_id"] == ITEM_ID
    assert set(ledger["requirements"]) == REQUIREMENTS
    assert set(ledger["decisions"]) == DECISIONS

    fixture_counts = ledger["fixture_counts"]
    assert set(fixture_counts) == EXPECTED_GROUPS
    for group, expected_count in observed_counts.items():
        group_record = fixture_counts[group]
        assert group_record["count"] == expected_count
        assert group_record["validation_suites"]
        assert group_record["item_ids"]
        for fixture_id in group_record["representative_fixture_ids"]:
            assert (compat_fixture_root / f"{fixture_id}.json").is_file()

    acceptance_slugs = {entry["slug"] for entry in ledger["acceptance_coverage"]}
    assert {
        "isolated-worktree-capture-provenance",
        "golden-fixture-domain-coverage",
        "observable-interface-tests",
        "python-pytest-guardrail",
        "first-failure-and-future-rust-triage",
        "fixture-storage-hygiene",
    } <= acceptance_slugs
    for entry in ledger["acceptance_coverage"]:
        assert entry["item_ids"]
        assert entry["validation_suites"]
        assert set(entry["fixture_groups"]) <= EXPECTED_GROUPS
        assert set(entry["requirement_ids"]) <= REQUIREMENTS
        assert set(entry["decision_ids"]) <= DECISIONS
        assert set(entry["decision_ids"])
        for fixture_id in entry["representative_fixture_ids"]:
            assert (compat_fixture_root / f"{fixture_id}.json").is_file()

    domain_slugs = {entry["slug"] for entry in ledger["domain_coverage"]}
    assert EXPECTED_DOMAIN_SLUGS <= domain_slugs

    contracts = ledger["contract_coverage"]
    assert set(contracts["requirements"]) == REQUIREMENTS
    assert set(contracts["decisions"]) == DECISIONS
    for section in ("requirements", "decisions"):
        for contract in contracts[section].values():
            assert set(contract["fixture_groups"]) == EXPECTED_GROUPS
            assert contract["validation_suites"]

    gap_ids = {gap["gap_id"] for gap in ledger["explicit_gaps"]}
    assert {
        "rust-parity-not-claimed",
        "installed-asset-closure",
        "production-trigger-runtime",
        "final-doc-validation",
    } <= gap_ids
    assert ledger["closure_evidence"]["no_python_behavior_retired"] is True


def test_validation_gate_records_commands_triage_rust_expectations_and_closure(
    compat_validation_root: Path,
) -> None:
    gate = _load_json(compat_validation_root / "m0-validation-gate.json")

    assert gate["schema_version"] == "compat-m0-validation-gate-v1"
    assert gate["milestone_id"] == MILESTONE_ID
    assert gate["active_item_id"] == ITEM_ID
    assert gate["no_python_behavior_retired"] is True
    assert set(gate["requirements"]) == REQUIREMENTS
    assert set(gate["decisions"]) == DECISIONS

    required = {entry["command"]: entry for entry in gate["required_commands"]}
    assert {
        "uv run pytest -q tests/compat",
        "uv run pytest -q",
        "npm --prefix frontend run test:unit",
    } <= set(required)
    for entry in required.values():
        assert entry["status"] in {"pending", "pass", "fail", "skipped"}
        assert entry["triage_command"]

    triage_commands = {entry["command"] for entry in gate["focused_python_triage_commands"]}
    assert "uv run pytest -q -x --maxfail=1 tests/compat" in triage_commands
    assert "uv run pytest -q -x --maxfail=1 tests/compat/test_m0_coverage_validation_gate.py" in triage_commands
    assert any("tests/compat/cli tests/compat/storage" in command for command in triage_commands)
    assert any("tests/compat/api tests/compat/live" in command for command in triage_commands)
    assert any("tests/compat/dsl tests/compat/transforms" in command for command in triage_commands)
    assert any("tests/contracts/frontend tests/compat/frontend-contracts tests/compat/packaging" in command for command in triage_commands)

    rust_commands = {entry["command"] for entry in gate["future_rust_equivalent_commands"]}
    assert "cargo fmt --all -- --check" in rust_commands
    assert "cargo check --workspace --all-targets" in rust_commands
    assert "cargo test --workspace --all-targets" in rust_commands
    assert any("spark-cli" in command for command in rust_commands)
    assert any("spark-http" in command for command in rust_commands)
    assert any("attractor-runtime" in command for command in rust_commands)
    assert any("packaging_compat" in command for command in rust_commands)

    claims = gate["later_milestone_claims"]
    assert claims["rust_parity_claimed"] is False
    assert claims["installed_asset_closure_claimed"] is False
    assert claims["production_trigger_runtime_claimed"] is False

    hygiene = gate["repository_hygiene"]
    assert hygiene["reviewed_fixture_root"].endswith("/compat-fixtures")
    assert hygiene["generated_roots_ignored"]
    assert hygiene["forbidden_untracked_generated_prefixes"]
    assert gate["closure_constraints"]


def test_generated_roots_are_ignored_and_not_visible_as_untracked_source(
    rewrite_worktree_path: Path,
    rewrite_runtime_dir: Path,
) -> None:
    runtime_rel = rewrite_runtime_dir.relative_to(rewrite_worktree_path).as_posix()
    ignore_candidates = [
        f"{runtime_rel}/validation/generated/coverage/probe.json",
        "tests/compat/_generated/coverage/output.json",
        "tests/compat/.tmp/coverage/output.json",
        "tests/compat/.server-logs/stdout.log",
        "frontend/src/__tests__/.tmp-compat-probes/probe.test.ts",
        "frontend/dist/assets/probe.js",
        "src/spark/ui_dist/index.html",
    ]
    for candidate in ignore_candidates:
        result = subprocess.run(
            ["git", "check-ignore", "-q", candidate],
            cwd=rewrite_worktree_path,
            text=True,
            capture_output=True,
            check=False,
        )
        assert result.returncode == 0, candidate

    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard"],
        cwd=rewrite_worktree_path,
        text=True,
        capture_output=True,
        check=False,
    )
    assert untracked.returncode == 0
    forbidden_prefixes = (
        ".spark/rust-rewrite/current/validation/generated/",
        "tests/compat/_generated/",
        "tests/compat/.tmp/",
        "tests/compat/.server-logs/",
        "frontend/src/__tests__/.tmp-compat-probes/",
        "frontend/dist/",
        "src/spark/ui_dist/",
    )
    for path in untracked.stdout.splitlines():
        assert not path.startswith(forbidden_prefixes), path


def _fixture_manifests(root: Path) -> list[tuple[Path, Mapping[str, Any]]]:
    manifests: list[tuple[Path, Mapping[str, Any]]] = []
    for path in sorted(root.rglob("*.json")):
        loaded = _load_json(path)
        manifests.append((path, loaded))
    return manifests


def _m0_fixture_manifests(root: Path) -> list[tuple[Path, Mapping[str, Any]]]:
    return [
        (path, manifest)
        for path, manifest in _fixture_manifests(root)
        if str(manifest.get("item_id", "")) in KNOWN_FIXTURE_ITEM_IDS
    ]


def _load_json(path: Path) -> dict[str, Any]:
    loaded = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(loaded, dict)
    return loaded
