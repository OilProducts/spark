from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Dict, List, Tuple

from attractor.dsl.models import DotAttribute, DotGraph, DotNode, DotValueType


_SELECTOR_RE = re.compile(r"([^{}]+)\{([^{}]*)\}", re.DOTALL)

_PROPERTY_MAP = {
    "model": "llm_model",
    "provider": "llm_provider",
    "reasoning_effort": "reasoning_effort",
    "llm_model": "llm_model",
    "llm_provider": "llm_provider",
}


@dataclass
class _StyleRule:
    selector: str
    properties: Dict[str, str]
    order: int


class ModelStylesheetTransform:
    def apply(self, graph: DotGraph) -> DotGraph:
        style_attr = graph.graph_attrs.get("model_stylesheet")
        if not style_attr:
            return graph

        stylesheet = style_attr.value
        if not isinstance(stylesheet, str) or stylesheet.strip() == "":
            return graph

        rules = _parse_rules(stylesheet)
        for node in graph.nodes.values():
            self._apply_rules(node, rules, style_attr.line)

        return graph

    def _apply_rules(self, node: DotNode, rules: List[_StyleRule], line: int) -> None:
        candidates: Dict[str, Tuple[int, int, str]] = {}
        for rule in rules:
            if not _selector_matches(rule.selector, node):
                continue
            specificity = _selector_specificity(rule.selector)
            for prop, value in rule.properties.items():
                current = candidates.get(prop)
                entry = (specificity, rule.order, value)
                if current is None or entry > current:
                    candidates[prop] = entry

        for prop, (_, _, value) in candidates.items():
            if prop in node.attrs:
                continue
            node.attrs[prop] = DotAttribute(
                key=prop,
                value=value,
                value_type=DotValueType.STRING,
                line=line,
            )


def _parse_rules(stylesheet: str) -> List[_StyleRule]:
    rules: List[_StyleRule] = []
    for idx, match in enumerate(_SELECTOR_RE.finditer(stylesheet)):
        selector = match.group(1).strip()
        body = match.group(2).strip()
        properties: Dict[str, str] = {}
        for stmt in [s.strip() for s in body.split(";") if s.strip()]:
            if ":" not in stmt:
                continue
            raw_key, raw_value = stmt.split(":", 1)
            key = raw_key.strip()
            value = _strip_quotes(raw_value.strip())
            mapped = _PROPERTY_MAP.get(key)
            if mapped:
                properties[mapped] = value
        if selector and properties:
            rules.append(_StyleRule(selector=selector, properties=properties, order=idx))
    return rules


def _selector_matches(selector: str, node: DotNode) -> bool:
    selector = selector.strip()
    if selector == "*":
        return True

    if selector.startswith("#"):
        return node.node_id == selector[1:]

    if selector.startswith("."):
        class_attr = node.attrs.get("class")
        if not class_attr:
            return False
        classes = [c.strip() for c in str(class_attr.value).split(",") if c.strip()]
        return selector[1:] in classes

    shape_attr = node.attrs.get("shape")
    return bool(shape_attr and str(shape_attr.value) == selector)


def _selector_specificity(selector: str) -> int:
    selector = selector.strip()
    if selector.startswith("#"):
        return 3
    if selector.startswith("."):
        return 2
    if selector == "*":
        return 0
    return 1


def _strip_quotes(value: str) -> str:
    if len(value) >= 2 and value[0] == '"' and value[-1] == '"':
        return value[1:-1]
    return value
