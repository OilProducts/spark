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

