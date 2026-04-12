from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Dict, List, Tuple

from attractor.engine.context import Context

RUNTIME_RETRY_NODE_ID_KEY = "_attractor.runtime.retry.node_id"
RUNTIME_RETRY_ATTEMPT_KEY = "_attractor.runtime.retry.attempt"
RUNTIME_RETRY_MAX_ATTEMPTS_KEY = "_attractor.runtime.retry.max_attempts"
RUNTIME_RETRY_FAILURE_REASON_KEY = "_attractor.runtime.retry.failure_reason"


@dataclass(frozen=True)
class RuntimePreambleTransform:
    node_outcomes_key: str = "_attractor.node_outcomes"

    def apply(
        self,
        fidelity: str,
        context: Context,
        completed_nodes: List[str],
        *,
        include_context_items: bool = True,
    ) -> str:
        mode = fidelity.strip().lower()
        if mode == "full":
            return ""

        snapshot = context.snapshot()
        goal = str(snapshot.get("graph.goal", "")).strip()
        run_id = str(snapshot.get("internal.run_id", "")).strip()
        statuses = snapshot.get(self.node_outcomes_key, {})
        if not isinstance(statuses, dict):
            statuses = {}
        retry_lines = self._retry_context_lines(snapshot)

        if mode == "truncate":
            lines = [
                "carryover:truncate",
                f"goal={goal}",
                f"run_id={run_id}",
            ]
            lines.extend(retry_lines)
            return "\n".join(lines)

        context_items = self._carryover_context_items(snapshot) if include_context_items else []
        if mode == "compact":
            lines = [
                "carryover:compact",
                f"goal={goal}",
                f"run_id={run_id}",
            ]
            if completed_nodes:
                completed_summary = ", ".join(
                    f"{node_id}:{statuses.get(node_id, '')}" for node_id in completed_nodes
                )
                lines.append(f"completed={completed_summary}")
            lines.extend(retry_lines)
            for key, value in context_items[:8]:
                lines.append(f"- {key}={value}")
            return "\n".join(lines)

        summary_limits = {
            "summary:low": (3, 4),
            "summary:medium": (6, 8),
            "summary:high": (12, 16),
        }
        if mode in summary_limits:
            max_stages, max_context_items = summary_limits[mode]
            recent_nodes = completed_nodes[-max_stages:]
            recent_summary = ", ".join(
                f"{node_id}:{statuses.get(node_id, '')}" for node_id in recent_nodes
            )
            lines = [
                f"carryover:{mode}",
                f"goal={goal}",
                f"run_id={run_id}",
                f"recent_stages={recent_summary}",
                f"log_entries={len(context.logs)}",
            ]
            lines.extend(retry_lines)
            for key, value in context_items[:max_context_items]:
                lines.append(f"{key}={value}")
            return "\n".join(lines)

        return self.apply("compact", context, completed_nodes)

    def _carryover_context_items(self, snapshot: Dict[str, object]) -> List[Tuple[str, str]]:
        items: List[Tuple[str, str]] = []
        for key in sorted(snapshot.keys()):
            if key in {"graph.goal", "internal.run_id"}:
                continue
            if key.startswith("_attractor.") or key.startswith("internal."):
                continue
            items.append((key, _to_context_text(snapshot[key])))
        return items

    def _retry_context_lines(self, snapshot: Dict[str, object]) -> List[str]:
        attempt_value = snapshot.get(RUNTIME_RETRY_ATTEMPT_KEY, 0)
        max_attempts_value = snapshot.get(RUNTIME_RETRY_MAX_ATTEMPTS_KEY, 0)
        attempt = _to_int(attempt_value)
        max_attempts = _to_int(max_attempts_value)
        if attempt <= 0 and max_attempts <= 0:
            return []

        lines = [
            f"retry.node_id={_to_context_text(snapshot.get(RUNTIME_RETRY_NODE_ID_KEY, ''))}",
            f"retry.attempt={attempt}",
            f"retry.max_attempts={max_attempts}",
        ]
        failure_reason = _to_context_text(snapshot.get(RUNTIME_RETRY_FAILURE_REASON_KEY, ""))
        if failure_reason:
            lines.append(f"retry.failure_reason={failure_reason}")
        return lines


def _to_context_text(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float, str)):
        return str(value)
    return json.dumps(value, sort_keys=True)


def _to_int(value: object) -> int:
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    try:
        return int(str(value).strip() or "0")
    except (TypeError, ValueError):
        return 0
