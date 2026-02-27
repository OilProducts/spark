from __future__ import annotations

import json
from pathlib import Path
from typing import Iterable, List


_ALLOWED_STATUS_OUTCOMES = {"success", "retry", "fail", "partial_success"}


def validate_artifact_and_status_contract(*, logs_root: Path, status_node_ids: Iterable[str]) -> List[str]:
    errors: List[str] = []
    artifacts_dir = logs_root / "artifacts"
    if not artifacts_dir.is_dir():
        errors.append(f"missing artifacts directory: {artifacts_dir}")

    for node_id in status_node_ids:
        status_path = logs_root / node_id / "status.json"
        if not status_path.is_file():
            errors.append(f"missing status file for node '{node_id}': {status_path}")
            continue

        try:
            payload = json.loads(status_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            errors.append(f"invalid JSON in status file for node '{node_id}': {exc}")
            continue

        if not isinstance(payload, dict):
            errors.append(f"status payload for node '{node_id}' must be an object")
            continue

        outcome = payload.get("outcome")
        if outcome not in _ALLOWED_STATUS_OUTCOMES:
            errors.append(
                f"invalid outcome for node '{node_id}': expected one of "
                f"{sorted(_ALLOWED_STATUS_OUTCOMES)}, got {outcome!r}"
            )

        preferred_next_label = payload.get("preferred_next_label")
        if "preferred_next_label" in payload and not isinstance(preferred_next_label, str):
            errors.append(f"preferred_next_label for node '{node_id}' must be a string")

        suggested_next_ids = payload.get("suggested_next_ids")
        if "suggested_next_ids" in payload and (
            not isinstance(suggested_next_ids, list)
            or any(not isinstance(candidate, str) for candidate in suggested_next_ids)
        ):
            errors.append(f"suggested_next_ids for node '{node_id}' must be a list of strings")

        context_updates = payload.get("context_updates")
        if "context_updates" in payload and not isinstance(context_updates, dict):
            errors.append(f"context_updates for node '{node_id}' must be an object")

        notes = payload.get("notes")
        if "notes" in payload and not isinstance(notes, str):
            errors.append(f"notes for node '{node_id}' must be a string")

    return errors
