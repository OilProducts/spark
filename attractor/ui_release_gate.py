from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
from typing import Iterable, Sequence, Tuple


@dataclass(frozen=True)
class RequiredUIFeatureRow:
    item_id: str
    description: str
    available: bool


_REQUIRED_UI_FEATURE_PATTERN = re.compile(
    r"^- \[(?P<status>[ x~])\] \[(?P<item_id>A[1-3]-\d+|B-\d+)\] (?P<description>.+)$"
)


def extract_required_ui_feature_rows(checklist_text: str) -> Tuple[RequiredUIFeatureRow, ...]:
    rows = []
    for raw_line in checklist_text.splitlines():
        match = _REQUIRED_UI_FEATURE_PATTERN.match(raw_line.strip())
        if not match:
            continue
        rows.append(
            RequiredUIFeatureRow(
                item_id=match.group("item_id"),
                description=match.group("description").strip(),
                available=match.group("status") == "x",
            )
        )
    return tuple(rows)


def enforce_required_ui_feature_release_gate(rows: Iterable[RequiredUIFeatureRow]) -> None:
    row_list = tuple(rows)
    if not row_list:
        raise RuntimeError("UI required-feature release gate failed: no required checklist items were found")

    unavailable = [row.item_id for row in row_list if not row.available]
    if unavailable:
        joined = ", ".join(unavailable)
        raise RuntimeError(f"UI required-feature release gate failed: unavailable checklist items: {joined}")


def run_required_ui_feature_release_gate(checklist_path: Path | str) -> Tuple[RequiredUIFeatureRow, ...]:
    checklist = Path(checklist_path)
    rows = extract_required_ui_feature_rows(checklist.read_text(encoding="utf-8"))
    enforce_required_ui_feature_release_gate(rows)
    return rows


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail release if any required UI feature checklist row is not marked complete."
    )
    parser.add_argument(
        "--checklist",
        default="ui-implementation-checklist.md",
        help="Path to ui-implementation-checklist.md",
    )
    args = parser.parse_args(argv)

    run_required_ui_feature_release_gate(args.checklist)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
