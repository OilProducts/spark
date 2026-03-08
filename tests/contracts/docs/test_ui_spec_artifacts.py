"""Documentation and spec artifact contracts for UI migration governance.

These tests validate documentation structure and traceability contracts rather
than exact prose wording, so editorial rewording does not cause false failures.
"""

from __future__ import annotations

import re
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
CHECKLIST_ITEM_RE = re.compile(r"(?m)^Checklist item:\s*\[([^\]]+)\]\s*$")


def _load_doc(doc_name: str) -> tuple[Path, str]:
    doc_path = REPO_ROOT / doc_name
    assert doc_path.exists(), f"Missing documentation artifact: {doc_path}"
    return doc_path, doc_path.read_text(encoding="utf-8")


def _checklist_item_id(doc_text: str) -> str:
    match = CHECKLIST_ITEM_RE.search(doc_text)
    assert match is not None, "Document missing checklist item marker."
    return match.group(1)


def _h2_headings(doc_text: str) -> list[str]:
    return re.findall(r"(?m)^##\s+(.+?)\s*$", doc_text)


def _section(doc_text: str, heading: str) -> str:
    pattern = rf"(?ms)^##\s+{re.escape(heading)}\s*$\n(.*?)(?=^##\s+|\Z)"
    match = re.search(pattern, doc_text)
    assert match is not None, f"Missing section: {heading}"
    return match.group(1).strip()


def _subsection(section_text: str, heading: str) -> str:
    pattern = rf"(?ms)^###\s+{re.escape(heading)}\s*$\n(.*?)(?=^###\s+|\Z)"
    match = re.search(pattern, section_text)
    assert match is not None, f"Missing subsection: {heading}"
    return match.group(1).strip()


def _numbered_items(section_text: str) -> list[str]:
    return [line.strip() for line in section_text.splitlines() if re.match(r"^\d+\.\s+", line.strip())]


def _bullet_items(section_text: str) -> list[str]:
    return [line.strip() for line in section_text.splitlines() if line.strip().startswith("- ")]


def _markdown_table(section_text: str) -> tuple[list[str], list[list[str]]]:
    lines: list[str] = []
    collecting = False
    for raw_line in section_text.splitlines():
        line = raw_line.strip()
        if line.startswith("|"):
            lines.append(line)
            collecting = True
            continue
        if collecting:
            break

    assert len(lines) >= 3, "Expected a markdown table with header, separator, and data rows."

    def parse_row(line: str) -> list[str]:
        return [cell.strip() for cell in line.strip("|").split("|")]

    header = parse_row(lines[0])
    rows = [parse_row(line) for line in lines[2:] if line.startswith("|")]
    assert rows, "Expected at least one data row in markdown table."
    return header, rows


def _extract_component_paths(cell_text: str) -> set[str]:
    return set(re.findall(r"frontend/src/components/[A-Za-z0-9_./-]+\.tsx", cell_text))


def _extract_ui_spec_refs(cell_text: str) -> set[str]:
    return set(re.findall(r"\b\d+\.\d+\b", cell_text))


def test_parity_complete_user_journey_doc_contract() -> None:
    _, doc_text = _load_doc("specs/ui-parity-complete-user-journey.md")
    assert _checklist_item_id(doc_text) == "1.2-01"

    headings = set(_h2_headings(doc_text))
    assert {"Preconditions", "Acceptance Script", "Expected Results"}.issubset(headings)

    preconditions = _section(doc_text, "Preconditions")
    acceptance_script = _section(doc_text, "Acceptance Script")
    expected_results = _section(doc_text, "Expected Results")
    assert len(_numbered_items(preconditions)) >= 3
    assert len(_numbered_items(acceptance_script)) >= 8
    assert len(_numbered_items(expected_results)) >= 3

    for phase in ("project-select", "author", "execute", "inspect"):
        assert phase in doc_text, f"Missing lifecycle phase marker: {phase}"


def test_parity_risk_report_doc_contract() -> None:
    _, doc_text = _load_doc("ui-parity-risk-report.md")
    assert _checklist_item_id(doc_text) == "1.1-02"

    headings = set(_h2_headings(doc_text))
    assert {
        "Scope",
        "Behavior-Loss Failure Modes",
        "Hidden-Config Failure Modes",
        "Notes",
    }.issubset(headings)

    scope = _section(doc_text, "Scope")
    assert "ui-spec.md" in scope
    assert "ui-raw-dot-required-config.md" in scope

    behavior_table_header, behavior_rows = _markdown_table(_section(doc_text, "Behavior-Loss Failure Modes"))
    assert behavior_table_header == [
        "Failure mode",
        "Trigger in current UI",
        "User-visible impact",
        "Severity",
        "Mitigation direction",
    ]
    assert len(behavior_rows) >= 3
    severities = {row[3] for row in behavior_rows}
    assert "High" in severities
    assert severities.issubset({"Low", "Medium", "High", "Critical"})

    hidden_table_header, hidden_rows = _markdown_table(_section(doc_text, "Hidden-Config Failure Modes"))
    assert hidden_table_header == [
        "Failure mode",
        "Hidden required config",
        "Discovery signal in current UI",
        "Severity",
        "Mitigation direction",
    ]
    assert len(hidden_rows) >= 3

    hidden_config_text = " ".join(row[1] for row in hidden_rows)
    for required_config in ("tool_hooks.pre", "tool_hooks.post", "manager.actions", "human.default_choice"):
        assert required_config in hidden_config_text, f"Missing hidden-config coverage for {required_config}"


def test_raw_dot_required_config_doc_contract() -> None:
    _, doc_text = _load_doc("ui-raw-dot-required-config.md")
    assert _checklist_item_id(doc_text) == "1.1-01"

    headings = set(_h2_headings(doc_text))
    assert {"Scope", "Current Required Raw-DOT Surfaces", "Notes"}.issubset(headings)

    table_header, rows = _markdown_table(_section(doc_text, "Current Required Raw-DOT Surfaces"))
    assert table_header == [
        "Spec anchor",
        "Required configuration",
        "Why raw DOT is currently required",
        "Evidence in current UI code",
    ]
    assert len(rows) >= 2

    for row in rows:
        assert "ui-spec.md" in row[0], "Spec anchor column must trace to ui-spec.md."
        assert "frontend/src/" in row[3], "Evidence column must cite frontend code locations."

    required_config_column = " ".join(row[1] for row in rows)
    assert "subgraph" in required_config_column
    assert "unknown-valid extension attributes" in required_config_column

    no_longer_required_attrs = {
        "stack.child_dotfile",
        "stack.child_workdir",
        "tool_hooks.pre",
        "tool_hooks.post",
        "manager.poll_interval",
        "manager.max_cycles",
        "manager.stop_condition",
        "manager.actions",
        "human.default_choice",
    }
    for attr in no_longer_required_attrs:
        assert attr not in required_config_column, f"Raw-DOT inventory still lists UI-supported attr: {attr}"


def test_role_persona_scenarios_doc_contract() -> None:
    _, doc_text = _load_doc("specs/ui-role-persona-scenarios.md")
    assert _checklist_item_id(doc_text) == "3.1-01"

    persona_headings = {
        "Pipeline Author Scenario",
        "Operator Scenario",
        "Reviewer/Auditor Scenario",
        "Project Owner/Planner Scenario",
    }
    assert persona_headings.issubset(set(_h2_headings(doc_text)))

    for persona in persona_headings:
        persona_section = _section(doc_text, persona)
        scenario = _subsection(persona_section, "Scenario")
        criteria = _subsection(persona_section, "Concrete UI Success Criteria")
        assert any(line.startswith("- Given") for line in scenario.splitlines())
        assert any(line.startswith("- When") for line in scenario.splitlines())
        assert any(line.startswith("- Then") for line in scenario.splitlines())
        assert len(_bullet_items(criteria)) >= 2


def test_runtime_parser_boundaries_doc_contract() -> None:
    _, doc_text = _load_doc("specs/ui-runtime-parser-boundaries.md")
    assert _checklist_item_id(doc_text) == "1.3-01"

    headings = set(_h2_headings(doc_text))
    assert {
        "Source of Truth",
        "UI-owned responsibilities",
        "Runtime/parser-owned responsibilities",
        "Boundary Rules",
    }.issubset(headings)

    source_of_truth = _section(doc_text, "Source of Truth")
    assert "ui-spec.md" in source_of_truth
    assert "attractor-spec.md" in source_of_truth

    ui_owned = _section(doc_text, "UI-owned responsibilities")
    runtime_owned = _section(doc_text, "Runtime/parser-owned responsibilities")
    boundary_rules = _section(doc_text, "Boundary Rules")
    assert len(_bullet_items(ui_owned)) >= 3
    assert len(_bullet_items(runtime_owned)) >= 3
    assert len(_numbered_items(boundary_rules)) >= 4

