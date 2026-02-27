from __future__ import annotations

import re

from .context import Context
from .outcome import Outcome


_CLAUSE_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_.]*)\s*(=|!=)\s*(.+)$")


def evaluate_condition(condition: str, outcome: Outcome, context: Context) -> bool:
    text = (condition or "").strip()
    if text == "":
        return True

    clauses = [clause.strip() for clause in _split_clauses(text)]
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
        prefixed = context.get(key, None)
        if prefixed is not None:
            return _normalize_value(_stringify(prefixed))
        return _normalize_value(context.get_context_path(key[len("context.") :]))
    return ""


def _split_clauses(condition: str) -> list[str]:
    clauses: list[str] = []
    current: list[str] = []
    in_quotes = False
    escaped = False
    index = 0

    while index < len(condition):
        char = condition[index]
        if escaped:
            current.append(char)
            escaped = False
            index += 1
            continue
        if char == "\\":
            current.append(char)
            escaped = True
            index += 1
            continue
        if char == '"':
            in_quotes = not in_quotes
            current.append(char)
            index += 1
            continue
        if not in_quotes and char == "&" and index + 1 < len(condition) and condition[index + 1] == "&":
            clauses.append("".join(current))
            current = []
            index += 2
            continue

        current.append(char)
        index += 1

    clauses.append("".join(current))
    return clauses


def _normalize_value(raw: str) -> str:
    text = raw.strip()
    if len(text) >= 2 and text[0] == '"' and text[-1] == '"':
        text = text[1:-1]
    return text


def _stringify(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)
