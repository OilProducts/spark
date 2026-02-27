from __future__ import annotations

from pathlib import Path

from attractor.parity_matrix import CROSS_FEATURE_PARITY_MATRIX_ROWS
from attractor.release_gate_traceability import build_release_gate_traceability_records


def _count_section_11_checkbox_items(spec_text: str) -> int:
    in_section_11 = False
    count = 0

    for line in spec_text.splitlines():
        stripped = line.strip()
        if stripped == "## 11. Definition of Done":
            in_section_11 = True
            continue
        if in_section_11 and stripped.startswith("## ") and not stripped.startswith("## 11."):
            break
        if in_section_11 and stripped.startswith("- [ ] "):
            count += 1

    return count


def test_release_gate_traceability_covers_section_11_and_parity_matrix() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    spec_path = repo_root / "attractor-spec.md"
    spec_text = spec_path.read_text(encoding="utf-8")

    records = build_release_gate_traceability_records(spec_text)
    section_11_records = [record for record in records if record.group == "section11"]
    parity_records = [record for record in records if record.group == "parity_matrix"]

    assert len(section_11_records) == _count_section_11_checkbox_items(spec_text)
    assert len(parity_records) == len(CROSS_FEATURE_PARITY_MATRIX_ROWS)
    assert not [record for record in records if not record.links]


def test_release_gate_traceability_links_reference_existing_artifacts() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    spec_text = (repo_root / "attractor-spec.md").read_text(encoding="utf-8")

    records = build_release_gate_traceability_records(spec_text)
    assert records

    for record in records:
        for link in record.links:
            path_part = link.reference.split("::", 1)[0]
            assert (repo_root / path_part).exists(), f"missing traceability artifact for {record.item}: {link.reference}"
            if link.kind == "automated":
                assert path_part.startswith("tests/")

