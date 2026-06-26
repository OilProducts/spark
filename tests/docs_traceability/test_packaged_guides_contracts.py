from __future__ import annotations

import re
import tomllib
from pathlib import Path

from attractor.dsl import parse_dot, validate_graph

from scripts import build_deliverable


REPO_ROOT = Path(__file__).resolve().parents[2]
GUIDE_AND_TUTORIAL_DOCS = (
    Path("docs/first-flow-tutorial.md"),
    Path("src/spark/guides/dot-authoring.md"),
    Path("src/spark/guides/spark-operations.md"),
)
PACKAGED_GUIDE_NAMES = ("dot-authoring.md", "spark-operations.md")
FENCE_RE = re.compile(r"(?ms)^```(?P<lang>[A-Za-z0-9_-]*)\s*$\n(?P<body>.*?)(?=^```\s*$)^```\s*$")
FLOW_REFERENCE_RE = re.compile(
    r"(?<![\w/.-])((?:src/spark/flows/)?(?:examples|software-development)/[A-Za-z0-9_./-]+\.dot)\b"
)


def test_complete_dot_examples_in_guides_parse_and_validate() -> None:
    examples = list(_complete_dot_examples())
    assert examples, "Expected at least one complete DOT example in guide docs."

    failures: list[str] = []
    for doc_path, source in examples:
        try:
            graph = parse_dot(source)
        except Exception as exc:  # noqa: BLE001 - assertion records the user-visible example.
            failures.append(f"{doc_path}: parse failed: {exc}")
            continue
        errors = [diagnostic.message for diagnostic in validate_graph(graph) if _is_error(diagnostic)]
        if errors:
            failures.append(f"{doc_path}: validation errors: {errors}")

    assert failures == []


def test_referenced_starter_flows_exist_parse_and_validate() -> None:
    references = sorted(_referenced_starter_flows())
    assert references, "Expected starter flow references in guide docs."

    failures: list[str] = []
    for doc_path, flow_name in references:
        flow_path = _starter_flow_path(flow_name)
        if not flow_path.exists():
            failures.append(f"{doc_path}: missing referenced flow {flow_name}")
            continue
        graph = parse_dot(flow_path.read_text(encoding="utf-8"))
        errors = [diagnostic.message for diagnostic in validate_graph(graph) if _is_error(diagnostic)]
        if errors:
            failures.append(f"{doc_path}: referenced flow {flow_name} has validation errors: {errors}")

    assert failures == []


def test_packaged_guide_metadata_includes_only_packaged_guides() -> None:
    pyproject = tomllib.loads((REPO_ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    spark_package_data = set(pyproject["tool"]["setuptools"]["package-data"]["spark"])

    assert "guides/*.md" in spark_package_data

    required_wheel_entries = set(build_deliverable.REQUIRED_WHEEL_ENTRIES)
    for guide_name in PACKAGED_GUIDE_NAMES:
        assert f"spark/guides/{guide_name}" in required_wheel_entries
        assert (REPO_ROOT / "src" / "spark" / "guides" / guide_name).is_file()

    assert "docs/first-flow-tutorial.md" not in required_wheel_entries
    assert set(build_deliverable.FORBIDDEN_WHEEL_ENTRIES) >= {
        "spark/guides/attractor-spec.md",
        "spark/guides/spark-flow-extensions.md",
    }


def _complete_dot_examples() -> list[tuple[Path, str]]:
    examples: list[tuple[Path, str]] = []
    for doc_path in GUIDE_AND_TUTORIAL_DOCS:
        text = (REPO_ROOT / doc_path).read_text(encoding="utf-8")
        for match in FENCE_RE.finditer(text):
            if match.group("lang").strip().lower() != "dot":
                continue
            body = match.group("body").strip()
            if body.startswith("digraph "):
                examples.append((doc_path, body))
    return examples


def _referenced_starter_flows() -> set[tuple[Path, str]]:
    references: set[tuple[Path, str]] = set()
    for doc_path in GUIDE_AND_TUTORIAL_DOCS:
        text = (REPO_ROOT / doc_path).read_text(encoding="utf-8")
        for match in FLOW_REFERENCE_RE.finditer(text):
            references.add((doc_path, match.group(1)))
    return references


def _starter_flow_path(flow_name: str) -> Path:
    flow_path = Path(flow_name)
    if flow_path.parts[:3] == ("src", "spark", "flows"):
        return REPO_ROOT / flow_path
    return REPO_ROOT / "src" / "spark" / "flows" / flow_path


def _is_error(diagnostic: object) -> bool:
    severity = getattr(diagnostic, "severity", "")
    severity_value = getattr(severity, "value", severity)
    return str(severity_value).lower() == "error"
