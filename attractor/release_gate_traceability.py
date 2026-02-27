from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Literal, Tuple

from attractor.parity_matrix import CROSS_FEATURE_PARITY_MATRIX_ROWS


TraceabilityKind = Literal["automated", "manual"]
TraceabilityGroup = Literal["section11", "parity_matrix"]


@dataclass(frozen=True)
class TraceabilityLink:
    kind: TraceabilityKind
    reference: str


@dataclass(frozen=True)
class TraceabilityRecord:
    record_id: str
    group: TraceabilityGroup
    section: str
    item: str
    links: Tuple[TraceabilityLink, ...]


def _automated_links(*references: str) -> Tuple[TraceabilityLink, ...]:
    return tuple(TraceabilityLink(kind="automated", reference=reference) for reference in references)


SECTION_11_AUTOMATED_LINKS: Dict[str, Tuple[TraceabilityLink, ...]] = {
    "11.1": _automated_links("tests/dsl/test_parser.py"),
    "11.2": _automated_links("tests/dsl/test_validator.py"),
    "11.3": _automated_links("tests/engine/test_executor.py", "tests/engine/test_routing.py"),
    "11.4": _automated_links("tests/engine/test_retry_goal_gate.py"),
    "11.5": _automated_links("tests/engine/test_retry_policy.py", "tests/engine/test_retry_goal_gate.py"),
    "11.6": _automated_links("tests/handlers/test_handlers.py"),
    "11.7": _automated_links(
        "tests/engine/test_context.py",
        "tests/engine/test_checkpointing.py",
        "tests/engine/test_artifact_store.py",
    ),
    "11.8": _automated_links("tests/interviewer/test_interviewer.py", "tests/interviewer/test_question_model.py"),
    "11.9": _automated_links("tests/engine/test_conditions.py"),
    "11.10": _automated_links("tests/transforms/test_transforms.py"),
    "11.11": _automated_links(
        "tests/transforms/test_transforms.py",
        "tests/api/test_pipeline_status_endpoint.py",
        "tests/api/test_pipeline_questions_endpoint.py",
    ),
    "11.13": _automated_links("tests/integration/test_integration_smoke_pipeline.py"),
}

PARITY_MATRIX_AUTOMATED_LINKS: Tuple[TraceabilityLink, ...] = _automated_links(
    "tests/integration/test_parity_matrix_report.py",
    "tests/integration/test_parity_matrix.py",
)


def build_release_gate_traceability_records(spec_text: str) -> Tuple[TraceabilityRecord, ...]:
    records = []
    in_section_11 = False
    current_subsection = ""
    subsection_counts: Dict[str, int] = {}

    for line in spec_text.splitlines():
        stripped = line.strip()
        if stripped == "## 11. Definition of Done":
            in_section_11 = True
            continue
        if in_section_11 and stripped.startswith("## ") and not stripped.startswith("## 11."):
            break
        if not in_section_11:
            continue

        if stripped.startswith("### 11."):
            current_subsection = stripped.removeprefix("### ").split(" ", 1)[0]
            continue

        if stripped.startswith("- [ ] "):
            subsection_counts[current_subsection] = subsection_counts.get(current_subsection, 0) + 1
            record_id = f"{current_subsection}-{subsection_counts[current_subsection]:02d}"
            item = stripped.removeprefix("- [ ] ").strip()
            links = SECTION_11_AUTOMATED_LINKS.get(current_subsection, ())
            records.append(
                TraceabilityRecord(
                    record_id=record_id,
                    group="section11",
                    section=current_subsection,
                    item=item,
                    links=links,
                )
            )

    for index, row_name in enumerate(CROSS_FEATURE_PARITY_MATRIX_ROWS, start=1):
        records.append(
            TraceabilityRecord(
                record_id=f"11.12-{index:02d}",
                group="parity_matrix",
                section="11.12",
                item=row_name,
                links=PARITY_MATRIX_AUTOMATED_LINKS,
            )
        )

    return tuple(records)

