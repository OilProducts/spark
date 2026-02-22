from __future__ import annotations

from typing import Iterable, Optional

from attractor.dsl.models import DotEdge

from .conditions import evaluate_condition
from .context import Context
from .outcome import Outcome


def select_next_edge(edges: Iterable[DotEdge], outcome: Outcome, context: Context) -> Optional[DotEdge]:
    ordered = list(edges)

    condition_matched = []
    for edge in ordered:
        condition = _attr_str(edge, "condition")
        if condition and evaluate_condition(condition, outcome, context):
            condition_matched.append(edge)
    if condition_matched:
        return _best_by_weight_then_lexical(condition_matched)

    preferred = outcome.preferred_label.strip()
    if preferred:
        norm_preferred = _normalize_label(preferred)
        for edge in ordered:
            condition = _attr_str(edge, "condition")
            if condition and not evaluate_condition(condition, outcome, context):
                continue
            if _normalize_label(_attr_str(edge, "label")) == norm_preferred:
                return edge

    if outcome.suggested_next_ids:
        for suggested_id in outcome.suggested_next_ids:
            for edge in ordered:
                condition = _attr_str(edge, "condition")
                if condition and not evaluate_condition(condition, outcome, context):
                    continue
                if edge.target == suggested_id:
                    return edge

    unconditional = [edge for edge in ordered if _attr_str(edge, "condition") == ""]
    if unconditional:
        return _best_by_weight_then_lexical(unconditional)

    return None


def _best_by_weight_then_lexical(edges: list[DotEdge]) -> Optional[DotEdge]:
    if not edges:
        return None
    return sorted(edges, key=lambda e: (-_attr_int(e, "weight", 0), e.target, e.source))[0]


def _attr_str(edge: DotEdge, key: str) -> str:
    attr = edge.attrs.get(key)
    if not attr:
        return ""
    return str(attr.value)


def _attr_int(edge: DotEdge, key: str, default: int) -> int:
    attr = edge.attrs.get(key)
    if not attr:
        return default
    value = attr.value
    if isinstance(value, int):
        return value
    try:
        return int(str(value))
    except ValueError:
        return default


def _normalize_label(label: str) -> str:
    text = (label or "").strip().lower()
    # Strip common accelerator prefixes: [Y] , Y) , Y -
    if text.startswith("[") and "]" in text:
        text = text[text.find("]") + 1 :].strip()
    if len(text) >= 2 and text[1] == ")" and text[0].isalnum():
        text = text[2:].strip()
    if len(text) >= 3 and text[0].isalnum() and text[1] == " " and text[2] == "-":
        text = text[3:].strip()
    return text
