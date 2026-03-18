from __future__ import annotations

from pathlib import Path

from attractor.dsl.dot_lint import (
    find_dot_paths,
    find_non_canonical_dot_diffs,
    find_start_node_lint_errors,
)


def test_flows_are_canonical_dot() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    starter_flows_dir = repo_root / "starter-flows"
    fixture_flows_dir = repo_root / "tests" / "fixtures" / "flows"
    dot_paths = find_dot_paths(starter_flows_dir) + find_dot_paths(fixture_flows_dir)

    assert dot_paths, "expected at least one .dot file under starter-flows/ or tests/fixtures/flows/"
    non_canonical = find_non_canonical_dot_diffs(dot_paths)

    assert not non_canonical, "non-canonical .dot files detected:\n" + "\n".join(non_canonical)


def test_find_dot_paths_recurses_into_subdirectories(tmp_path: Path) -> None:
    top = tmp_path / "top.dot"
    nested = tmp_path / "nested" / "child.dot"
    nested.parent.mkdir(parents=True, exist_ok=True)
    top.write_text("digraph G { a -> b; }\n", encoding="utf-8")
    nested.write_text("digraph G { b -> c; }\n", encoding="utf-8")

    found = find_dot_paths(tmp_path)

    assert found == [nested, top]


def test_lint_reports_start_node_cardinality_violations(tmp_path: Path) -> None:
    missing_start = tmp_path / "missing_start.dot"
    missing_start.write_text(
        "digraph G { task [shape=box]; done [shape=Msquare]; task -> done; }\n",
        encoding="utf-8",
    )

    errors = find_start_node_lint_errors([missing_start])

    assert len(errors) == 1
    assert "start_node" in errors[0]
    assert "exactly one start node" in errors[0]


def test_justfile_exposes_dot_lint_recipe() -> None:
    justfile = Path(__file__).resolve().parents[2] / "justfile"
    content = justfile.read_text(encoding="utf-8")

    assert "\ndot-lint:\n" in f"\n{content}"
    assert "uv run pytest -q tests/repo_hygiene/test_dot_format_lint.py" in content


def test_ci_runs_dot_lint() -> None:
    workflows_dir = Path(__file__).resolve().parents[2] / ".github" / "workflows"
    workflow_paths = sorted(workflows_dir.glob("*.yml")) + sorted(workflows_dir.glob("*.yaml"))

    assert workflow_paths, "expected at least one CI workflow under .github/workflows/"

    has_dot_lint_step = False
    for path in workflow_paths:
        content = path.read_text(encoding="utf-8")
        if "just dot-lint" in content or "tests/repo_hygiene/test_dot_format_lint.py" in content:
            has_dot_lint_step = True
            break

    assert has_dot_lint_step, "expected CI workflow to run DOT lint check"


def test_ci_runs_parser_unsupported_grammar_regression_suite() -> None:
    justfile = Path(__file__).resolve().parents[2] / "justfile"
    justfile_content = justfile.read_text(encoding="utf-8")

    assert "\nparser-unsupported-grammar:\n" in f"\n{justfile_content}"
    assert "uv run pytest -q tests/dsl/test_parser.py -k unsupported_grammar_regression" in justfile_content

    workflows_dir = Path(__file__).resolve().parents[2] / ".github" / "workflows"
    workflow_paths = sorted(workflows_dir.glob("*.yml")) + sorted(workflows_dir.glob("*.yaml"))

    assert workflow_paths, "expected at least one CI workflow under .github/workflows/"

    has_parser_guard_step = False
    for path in workflow_paths:
        content = path.read_text(encoding="utf-8")
        if "just parser-unsupported-grammar" in content:
            has_parser_guard_step = True
            break

    assert has_parser_guard_step, "expected CI workflow to run parser unsupported-grammar regression suite"
