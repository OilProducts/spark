from __future__ import annotations

import re

from .context import Context
from .outcome import Outcome


_CLAUSE_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_.]*)\s*(=|!=)\s*(.+)$")


def evaluate_condition(condition: str, outcome: Outcome, context: Context) -> bool:
    text = (condition or "").strip()
    if text == "":
        return True

    clauses = [clause.strip() for clause in text.split("&&")]
    for clause in clauses:
        if clause == "":
            return False
        match = _CLAUSE_RE.match(clause)
        if not match:
            return False

        key = match.group(1)
        op = match.group(2)
        expected = _normalize_value(match.group(3))
        actual = _resolve_key(key, outcome, context)

        if op == "=" and actual != expected:
            return False
        if op == "!=" and actual == expected:
            return False

    return True


def _resolve_key(key: str, outcome: Outcome, context: Context) -> str:
    if key == "outcome":
        return outcome.status.value
    if key == "preferred_label":
        return _normalize_value(outcome.preferred_label)
    if key.startswith("context."):
        return _normalize_value(context.get_context_path(key[len("context.") :]))
    return ""


def _normalize_value(raw: str) -> str:
    text = raw.strip()
    if len(text) >= 2 and text[0] == '"' and text[-1] == '"':
        text = text[1:-1]
    return text
