from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Dict, List, Tuple

from attractor.dsl.models import DotAttribute, DotGraph, DotNode, DotValueType


_ALLOWED_PROPERTIES = {"llm_model", "llm_provider", "reasoning_effort"}
_CLASS_NAME_RE = re.compile(r"^[a-z0-9-]+$")
_NODE_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


@dataclass
class _StyleRule:
    selector: str
    properties: Dict[str, str]
    order: int


class ModelStylesheetTransform:
    def apply(self, graph: DotGraph) -> DotGraph:
        style_attr = graph.graph_attrs.get("model_stylesheet")
        rules: List[_StyleRule] = []
        stylesheet_line = 0
        if style_attr and isinstance(style_attr.value, str) and style_attr.value.strip() != "":
            rules = _parse_rules(style_attr.value)
            stylesheet_line = style_attr.line

        graph_defaults = _graph_default_model_attrs(graph)
        for node in graph.nodes.values():
            self._apply_rules(node, rules, stylesheet_line, graph_defaults)

        return graph

    def _apply_rules(
        self,
        node: DotNode,
        rules: List[_StyleRule],
        line: int,
        graph_defaults: Dict[str, Tuple[str, int]],
    ) -> None:
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

        for prop in _ALLOWED_PROPERTIES:
            existing = node.attrs.get(prop)
            # Preserve explicit per-node attributes from source; allow stylesheet
            # to replace parser/default-injected placeholders and inherited node defaults.
            if existing is not None and _is_explicit_node_attr(existing, node):
                continue
            if prop in candidates:
                value = candidates[prop][2]
                value_line = line
            elif prop in graph_defaults:
                value, value_line = graph_defaults[prop]
            else:
                continue
            node.attrs[prop] = DotAttribute(
                key=prop,
                value=value,
                value_type=DotValueType.STRING,
                line=value_line,
            )


def _parse_rules(stylesheet: str) -> List[_StyleRule]:
    text = stylesheet.strip()
    rules: List[_StyleRule] = []
    idx = 0
    order = 0
    n = len(text)
    while idx < n:
        while idx < n and text[idx].isspace():
            idx += 1
        if idx >= n:
            break

        brace = _find_unquoted(text, "{", idx)
        if brace == -1:
            break

        selector = text[idx:brace].strip()
        close = _find_unquoted(text, "}", brace + 1)
        if close == -1:
            break
        body = text[brace + 1 : close].strip()

        properties: Dict[str, str] = {}
        rule_is_valid = _selector_is_valid(selector)
        for statement in _split_unquoted(body, ";"):
            stmt = statement.strip()
            if not stmt:
                continue
            colon_count = _count_unquoted(stmt, ":")
            if colon_count != 1:
                rule_is_valid = False
                break
            colon = _find_unquoted(stmt, ":")
            raw_key = stmt[:colon]
            raw_value = stmt[colon + 1 :]
            key = raw_key.strip()
            value = _strip_quotes(raw_value.strip())
            if key not in _ALLOWED_PROPERTIES or value == "":
                rule_is_valid = False
                break
            properties[key] = value
        if selector and properties and rule_is_valid:
            rules.append(_StyleRule(selector=selector, properties=properties, order=order))
        order += 1
        idx = close + 1
    return rules


def _selector_is_valid(selector: str) -> bool:
    if selector == "*":
        return True
    if selector.startswith("."):
        return bool(_CLASS_NAME_RE.fullmatch(selector[1:] or ""))
    if selector.startswith("#"):
        return bool(_NODE_ID_RE.fullmatch(selector[1:] or ""))
    return False


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

    return False


def _selector_specificity(selector: str) -> int:
    selector = selector.strip()
    if selector.startswith("#"):
        return 3
    if selector.startswith("."):
        return 2
    if selector == "*":
        return 0
    return -1


def _strip_quotes(value: str) -> str:
    if len(value) >= 2 and value[0] == '"' and value[-1] == '"':
        return value[1:-1]
    return value


def _find_unquoted(text: str, token: str, start: int = 0) -> int:
    in_quotes = False
    escaped = False
    for idx in range(start, len(text)):
        char = text[idx]
        if char == "\\" and in_quotes and not escaped:
            escaped = True
            continue
        if char == '"' and not escaped:
            in_quotes = not in_quotes
        elif char == token and not in_quotes:
            return idx
        escaped = False
    return -1


def _count_unquoted(text: str, token: str) -> int:
    count = 0
    in_quotes = False
    escaped = False
    for char in text:
        if char == "\\" and in_quotes and not escaped:
            escaped = True
            continue
        if char == '"' and not escaped:
            in_quotes = not in_quotes
        elif char == token and not in_quotes:
            count += 1
        escaped = False
    return count


def _split_unquoted(text: str, token: str) -> List[str]:
    parts: List[str] = []
    start = 0
    in_quotes = False
    escaped = False
    for idx, char in enumerate(text):
        if char == "\\" and in_quotes and not escaped:
            escaped = True
            continue
        if char == '"' and not escaped:
            in_quotes = not in_quotes
        elif char == token and not in_quotes:
            parts.append(text[start:idx])
            start = idx + 1
        escaped = False
    parts.append(text[start:])
    return parts


def _graph_default_model_attrs(graph: DotGraph) -> Dict[str, Tuple[str, int]]:
    defaults: Dict[str, Tuple[str, int]] = {}
    for prop in _ALLOWED_PROPERTIES:
        attr = graph.graph_attrs.get(prop)
        if not attr or attr.line <= 0:
            continue
        value = str(attr.value).strip()
        if value == "":
            continue
        defaults[prop] = (value, attr.line)
    return defaults


def _is_explicit_node_attr(attr: DotAttribute, node: DotNode) -> bool:
    if attr.line <= 0:
        return False
    # Attributes inherited from `node [...]` defaults appear on an earlier line
    # than the node declaration itself; treat those as overridable defaults.
    if node.line > 0 and attr.line < node.line:
        return False
    return True
