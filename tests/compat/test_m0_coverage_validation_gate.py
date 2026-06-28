from __future__ import annotations

from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess
from typing import Any, Iterable, Mapping

import pytest

from tests.compat import harness


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
EXPECTED_ITEM_GROUPS = {
    "M0-I02-CLI-FILESYSTEM-FIXTURES": {"cli", "filesystem"},
    "M0-I03-HTTP-SSE-FIXTURES": {"http", "sse"},
    "M0-I04-DSL-RUNTIME-FIXTURES": {"dsl", "runtime"},
    "M0-I05-FRONTEND-PACKAGING-FIXTURES": {"frontend", "packaging"},
}
FIXTURE_ROOT_RELATIVE = "tests/compat/fixtures"
FORBIDDEN_COMMITTED_FIXTURE_PATTERNS = {
    "ignored rust-rewrite current alias": re.compile(r"\.spark/rust-rewrite/current"),
    "legacy compat current provenance token": re.compile(r"__LEGACY_COMPAT_CURRENT__"),
    "legacy compat runtime provenance token": re.compile(r"__LEGACY_COMPAT_RUNTIME__"),
    "machine-local home path": re.compile(r"/home/[^/\s]+/"),
    "machine-local macOS home path": re.compile(r"/Users/[^/\s]+/"),
    "pytest compat temp root": re.compile(r"/tmp/spark-compat-[^/\s]+"),
    "local Codex package path": re.compile(r"node_modules/@openai/codex"),
}


def test_reviewed_compat_fixtures_are_tracked_repository_inputs(
    rewrite_worktree_path: Path,
    compat_fixture_root: Path,
) -> None:
    tracked_paths = set(_git_ls_files(rewrite_worktree_path))
    tracked_fixture_paths = sorted(
        path for path in tracked_paths if path.startswith(f"{FIXTURE_ROOT_RELATIVE}/")
    )
    fixture_paths = sorted(
        path.relative_to(rewrite_worktree_path).as_posix()
        for path in compat_fixture_root.rglob("*")
        if path.is_file()
    )

    assert tracked_fixture_paths
    assert set(fixture_paths) <= tracked_paths


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
    assert item_groups == EXPECTED_ITEM_GROUPS


def test_committed_m0_fixture_manifests_cover_required_contracts_by_group(
    compat_fixture_root: Path,
) -> None:
    manifests = _m0_fixture_manifests(compat_fixture_root)
    grouped: dict[str, list[Mapping[str, Any]]] = defaultdict(list)

    for path, manifest in manifests:
        fixture_id = str(manifest.get("fixture_id", ""))
        assert compat_fixture_root.joinpath(f"{fixture_id}.json") == path
        grouped[path.relative_to(compat_fixture_root).parts[0]].append(manifest)

    assert set(grouped) == EXPECTED_GROUPS
    for group, group_manifests in grouped.items():
        assert group_manifests
        assert {
            requirement
            for manifest in group_manifests
            for requirement in manifest.get("requirements", [])
        } >= REQUIREMENTS, group
        assert {
            decision
            for manifest in group_manifests
            for decision in manifest.get("decisions", [])
        } >= DECISIONS, group


def test_public_manifest_coverage_helper_rejects_missing_contracts() -> None:
    manifest = {
        "fixture_id": "coverage/probe",
        "requirements": sorted(REQUIREMENTS),
        "decisions": sorted(DECISIONS),
    }

    coverage = harness.validate_manifest_coverage(
        manifest,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )

    assert set(coverage["requirements"]) == REQUIREMENTS
    assert set(coverage["decisions"]) == DECISIONS

    with pytest.raises(AssertionError):
        harness.validate_manifest_coverage(
            {**manifest, "decisions": ["CD-RR-001"]},
            requirement_ids=REQUIREMENTS,
            decision_ids=DECISIONS,
        )


def test_committed_compat_fixtures_do_not_embed_ignored_current_runtime(
    compat_fixture_root: Path,
) -> None:
    offenders: list[str] = []
    for path, manifest in _fixture_manifests(compat_fixture_root):
        relative = path.relative_to(compat_fixture_root)
        for value in _string_values(manifest):
            for label, pattern in FORBIDDEN_COMMITTED_FIXTURE_PATTERNS.items():
                if pattern.search(value):
                    offenders.append(f"{relative}: {label}: {value}")

    assert offenders == []


def test_generated_roots_are_ignored_and_not_visible_as_untracked_source(
    rewrite_worktree_path: Path,
) -> None:
    ignore_candidates = [
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


def _git_ls_files(root: Path) -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=root,
        check=True,
        text=True,
        capture_output=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def _string_values(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, Mapping):
        for nested in value.values():
            yield from _string_values(nested)
    elif isinstance(value, list):
        for nested in value:
            yield from _string_values(nested)
