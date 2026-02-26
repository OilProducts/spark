from __future__ import annotations

from difflib import unified_diff
from pathlib import Path

from attractor.dsl.formatter import canonicalize_dot


def test_flows_are_canonical_dot() -> None:
    flows_dir = Path(__file__).resolve().parents[2] / "flows"
    dot_paths = sorted(flows_dir.glob("*.dot"))

    assert dot_paths, "expected at least one .dot file under flows/"

    non_canonical: list[str] = []

    for path in dot_paths:
        source = path.read_text(encoding="utf-8")
        canonical = canonicalize_dot(source)
        normalized_source = source if source.endswith("\n") else f"{source}\n"
        if normalized_source != canonical:
            diff = "".join(
                unified_diff(
                    normalized_source.splitlines(keepends=True),
                    canonical.splitlines(keepends=True),
                    fromfile=f"{path} (current)",
                    tofile=f"{path} (canonical)",
                )
            )
            non_canonical.append(diff)

    assert not non_canonical, "non-canonical .dot files detected:\n" + "\n".join(non_canonical)


def test_justfile_exposes_dot_lint_recipe() -> None:
    justfile = Path(__file__).resolve().parents[2] / "justfile"
    content = justfile.read_text(encoding="utf-8")

    assert "\ndot-lint:\n" in f"\n{content}"
    assert "uv run pytest -q tests/integration/test_dot_format_lint.py" in content


def test_ci_runs_dot_lint() -> None:
    workflows_dir = Path(__file__).resolve().parents[2] / ".github" / "workflows"
    workflow_paths = sorted(workflows_dir.glob("*.yml")) + sorted(workflows_dir.glob("*.yaml"))

    assert workflow_paths, "expected at least one CI workflow under .github/workflows/"

    has_dot_lint_step = False
    for path in workflow_paths:
        content = path.read_text(encoding="utf-8")
        if "just dot-lint" in content or "tests/integration/test_dot_format_lint.py" in content:
            has_dot_lint_step = True
            break

    assert has_dot_lint_step, "expected CI workflow to run DOT lint check"
