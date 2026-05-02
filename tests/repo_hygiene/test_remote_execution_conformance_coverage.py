from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
COVERAGE_PATH = Path(__file__).with_name("remote_execution_conformance_coverage.json")

EXPECTED_REQUIREMENTS = {f"REQ-{number:03d}" for number in range(1, 15)}
EXPECTED_DECISIONS = {f"DEC-{number:03d}" for number in range(1, 16)}
ALLOWED_TARGET_ROOTS = (
    Path("tests/execution"),
    Path("tests/api"),
    Path("tests/handlers"),
    Path("tests/contracts/frontend"),
    Path("tests/dsl"),
    Path("tests/repo_hygiene"),
    Path("tests/test_cli.py"),
    Path("tests/test_sse.py"),
)
OBSERVABLE_SURFACES = {
    "CLI launch payload",
    "control-plane API routes",
    "control-plane callback behavior",
    "control-plane child-run behavior",
    "control-plane cleanup behavior",
    "control-plane remote runner state",
    "DSL validation",
    "filesystem configuration loading",
    "filesystem effects",
    "frontend API contract",
    "package boundary",
    "path mapping behavior",
    "pipeline API and run record state",
    "pipeline context API",
    "pipeline launch API",
    "project metadata API",
    "remote admission request and launch metadata",
    "remote client launch validation",
    "run-node process protocol",
    "settings API response",
    "spark-server CLI",
    "worker HTTP API",
    "worker HTTP API and event state",
    "worker SSE API",
    "worker snapshot state",
}


def test_remote_execution_conformance_baseline_covers_every_requirement_and_decision() -> None:
    baseline = _load_baseline()

    requirement_ids = {entry["id"] for entry in baseline["requirements"]}
    decision_ids = {entry["id"] for entry in baseline["decisions"]}

    assert requirement_ids == EXPECTED_REQUIREMENTS
    assert decision_ids == EXPECTED_DECISIONS
    assert baseline["gaps"] == []


def test_remote_execution_conformance_evidence_points_to_behavior_tests() -> None:
    baseline = _load_baseline()

    selectors: set[str] = set()
    for requirement in baseline["requirements"]:
        evidence = requirement["evidence"]
        assert evidence
        for item in evidence:
            assert item["surface"] in OBSERVABLE_SURFACES
            assert item["validates"].strip()
            selectors.add(item["selector"])

    for decision in baseline["decisions"]:
        evidence = decision["evidence"]
        assert evidence
        selectors.update(evidence)

    for selector in sorted(selectors):
        _assert_selector_targets_allowed_test(selector)
        _assert_selector_resolves_to_test_object(selector)


def _load_baseline() -> dict[str, Any]:
    return json.loads(COVERAGE_PATH.read_text(encoding="utf-8"))


def _assert_selector_targets_allowed_test(selector: str) -> None:
    test_path = Path(selector.split("::", 1)[0])
    assert any(test_path == root or root in test_path.parents for root in ALLOWED_TARGET_ROOTS), selector


def _assert_selector_resolves_to_test_object(selector: str) -> None:
    path_text, *object_parts = selector.split("::")
    assert object_parts, selector

    module_path = REPO_ROOT / path_text
    assert module_path.exists(), selector

    module_name = "conformance_coverage_" + "_".join(module_path.with_suffix("").parts[-4:])
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    assert spec is not None and spec.loader is not None, selector

    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    target: Any = module
    for part in object_parts:
        assert hasattr(target, part), selector
        target = getattr(target, part)
