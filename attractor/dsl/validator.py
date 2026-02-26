from __future__ import annotations

import re
from typing import Dict, Iterable, List, Set

from .models import Diagnostic, DiagnosticSeverity, DotEdge, DotGraph, DotNode


VALID_FIDELITY = {
    "full",
    "truncate",
    "compact",
    "summary:low",
    "summary:medium",
    "summary:high",
}

_CONDITION_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_.]*)\s*(=|!=)\s*(.+)$")
_STYLESHEET_ALLOWED_PROPERTIES = {"llm_model", "llm_provider", "reasoning_effort"}
_STYLESHEET_CLASS_RE = re.compile(r"^[a-z0-9-]+$")
_STYLESHEET_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def validate_graph(graph: DotGraph) -> List[Diagnostic]:
    diagnostics: List[Diagnostic] = []

    start_nodes = _find_start_nodes(graph)
    exit_nodes = _find_exit_nodes(graph)

    if len(start_nodes) != 1:
        if not start_nodes:
            diagnostics.append(
                Diagnostic(
                    rule_id="start_node",
                    severity=DiagnosticSeverity.ERROR,
                    message="pipeline must have exactly one start node, found 0",
                    line=0,
                )
            )
        else:
            for node in start_nodes:
                diagnostics.append(
                    Diagnostic(
                        rule_id="start_node",
                        severity=DiagnosticSeverity.ERROR,
                        message=f"pipeline must have exactly one start node, found {len(start_nodes)}",
                        line=node.line,
                        node_id=node.node_id,
                    )
                )

    if len(exit_nodes) != 1:
        if not exit_nodes:
            diagnostics.append(
                Diagnostic(
                    rule_id="terminal_node",
                    severity=DiagnosticSeverity.ERROR,
                    message="pipeline must have exactly one exit node, found 0",
                    line=0,
                )
            )
        else:
            for node in exit_nodes:
                diagnostics.append(
                    Diagnostic(
                        rule_id="terminal_node",
                        severity=DiagnosticSeverity.ERROR,
                        message=f"pipeline must have exactly one exit node, found {len(exit_nodes)}",
                        line=node.line,
                        node_id=node.node_id,
                    )
                )

    # Edge targets and start/exit in/out checks.
    in_degree: Dict[str, int] = {node_id: 0 for node_id in graph.nodes}
    out_degree: Dict[str, int] = {node_id: 0 for node_id in graph.nodes}

    for edge in graph.edges:
        if edge.source not in graph.nodes:
            diagnostics.append(
                Diagnostic(
                    rule_id="edge_source_exists",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"edge source '{edge.source}' does not reference an existing node",
                    line=edge.line,
                    edge=(edge.source, edge.target),
                )
            )
        if edge.target not in graph.nodes:
            diagnostics.append(
                Diagnostic(
                    rule_id="edge_target_exists",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"edge target '{edge.target}' does not reference an existing node",
                    line=edge.line,
                    edge=(edge.source, edge.target),
                )
            )

        if edge.source in out_degree:
            out_degree[edge.source] += 1
        if edge.target in in_degree:
            in_degree[edge.target] += 1

        diagnostics.extend(_validate_edge_condition(edge.attrs.get("condition"), edge))

    if len(start_nodes) == 1:
        start = start_nodes[0]
        if in_degree.get(start.node_id, 0) != 0:
            diagnostics.append(
                Diagnostic(
                    rule_id="start_no_incoming",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"start node '{start.node_id}' must have no incoming edges",
                    line=start.line,
                    node_id=start.node_id,
                )
            )

    for exit_node in exit_nodes:
        if out_degree.get(exit_node.node_id, 0) != 0:
            diagnostics.append(
                Diagnostic(
                    rule_id="exit_no_outgoing",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"exit node '{exit_node.node_id}' must have no outgoing edges",
                    line=exit_node.line,
                    node_id=exit_node.node_id,
                )
            )

    exit_node_ids = {node.node_id for node in exit_nodes}
    for node in graph.nodes.values():
        if node.node_id in exit_node_ids:
            continue
        if out_degree.get(node.node_id, 0) == 0:
            diagnostics.append(
                Diagnostic(
                    rule_id="node_has_outgoing_edge",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"node '{node.node_id}' must declare at least one outgoing edge",
                    line=node.line,
                    node_id=node.node_id,
                )
            )

    if len(start_nodes) == 1:
        reachable = _reachable_nodes(graph, start_nodes[0].node_id)
        for node_id, node in graph.nodes.items():
            if node_id not in reachable:
                diagnostics.append(
                    Diagnostic(
                        rule_id="reachability",
                        severity=DiagnosticSeverity.ERROR,
                        message=f"node '{node_id}' is not reachable from start node",
                        line=node.line,
                        node_id=node.node_id,
                    )
                )

    diagnostics.extend(_validate_retry_targets(graph))
    diagnostics.extend(_validate_fidelity_values(graph))
    diagnostics.extend(_validate_stylesheet(graph))

    return diagnostics


def _find_start_nodes(graph: DotGraph) -> List[DotNode]:
    shape_nodes: List[DotNode] = []
    fallback_nodes: List[DotNode] = []
    for node in graph.nodes.values():
        shape = _attr_str(node.attrs, "shape")
        if shape == "Mdiamond":
            shape_nodes.append(node)
        elif node.node_id in {"start", "Start"}:
            fallback_nodes.append(node)
    return shape_nodes or fallback_nodes


def _find_exit_nodes(graph: DotGraph) -> List[DotNode]:
    shape_nodes: List[DotNode] = []
    fallback_nodes: List[DotNode] = []
    for node in graph.nodes.values():
        shape = _attr_str(node.attrs, "shape")
        if shape == "Msquare":
            shape_nodes.append(node)
        elif node.node_id in {"exit", "end", "Exit", "End"}:
            fallback_nodes.append(node)
    return shape_nodes or fallback_nodes


def _attr_str(attrs: Dict[str, object], key: str) -> str:
    attr = attrs.get(key)
    if not attr:
        return ""
    return str(getattr(attr, "value", ""))


def _reachable_nodes(graph: DotGraph, start_id: str) -> Set[str]:
    adjacency: Dict[str, List[str]] = {node_id: [] for node_id in graph.nodes}
    for edge in graph.edges:
        if edge.source in adjacency:
            adjacency[edge.source].append(edge.target)

    visited: Set[str] = set()
    stack = [start_id]
    while stack:
        cur = stack.pop()
        if cur in visited:
            continue
        visited.add(cur)
        for nxt in adjacency.get(cur, []):
            if nxt not in visited:
                stack.append(nxt)
    return visited


def _validate_edge_condition(condition_attr, edge: DotEdge) -> List[Diagnostic]:
    diagnostics: List[Diagnostic] = []
    if not condition_attr:
        return diagnostics

    condition = condition_attr.value
    if not isinstance(condition, str):
        diagnostics.append(
            Diagnostic(
                rule_id="condition_syntax",
                severity=DiagnosticSeverity.ERROR,
                message="edge condition must be a string",
                line=edge.line,
                edge=(edge.source, edge.target),
            )
        )
        return diagnostics

    stripped = condition.strip()
    if stripped == "":
        return diagnostics

    clauses = [cl.strip() for cl in stripped.split("&&")]
    for clause in clauses:
        if clause == "":
            diagnostics.append(
                Diagnostic(
                    rule_id="condition_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message="empty condition clause is not allowed",
                    line=edge.line,
                    edge=(edge.source, edge.target),
                )
            )
            continue

        match = _CONDITION_RE.match(clause)
        if not match:
            diagnostics.append(
                Diagnostic(
                    rule_id="condition_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"invalid condition clause '{clause}'",
                    line=edge.line,
                    edge=(edge.source, edge.target),
                )
            )
            continue

        key = match.group(1)
        if key not in {"outcome", "preferred_label"} and not key.startswith("context."):
            diagnostics.append(
                Diagnostic(
                    rule_id="condition_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message=f"invalid condition variable '{key}'",
                    line=edge.line,
                    edge=(edge.source, edge.target),
                )
            )

    return diagnostics


def _validate_retry_targets(graph: DotGraph) -> List[Diagnostic]:
    diagnostics: List[Diagnostic] = []

    graph_retry_keys = ["retry_target", "fallback_retry_target"]
    for key in graph_retry_keys:
        attr = graph.graph_attrs.get(key)
        if not attr:
            continue
        target = str(attr.value)
        if target and target not in graph.nodes:
            diagnostics.append(
                Diagnostic(
                    rule_id="retry_target_exists",
                    severity=DiagnosticSeverity.WARNING,
                    message=f"graph attribute {key} references missing node '{target}'",
                    line=attr.line,
                )
            )

    for node in graph.nodes.values():
        for key in graph_retry_keys:
            attr = node.attrs.get(key)
            if not attr:
                continue
            target = str(attr.value)
            if target and target not in graph.nodes:
                diagnostics.append(
                    Diagnostic(
                        rule_id="retry_target_exists",
                        severity=DiagnosticSeverity.WARNING,
                        message=f"node '{node.node_id}' {key} references missing node '{target}'",
                        line=attr.line,
                        node_id=node.node_id,
                    )
                )

    return diagnostics


def _validate_fidelity_values(graph: DotGraph) -> List[Diagnostic]:
    diagnostics: List[Diagnostic] = []

    def check_attr(rule_owner: str, attr, line: int, node_id: str | None = None, edge: tuple[str, str] | None = None):
        if not attr:
            return
        value = str(attr.value)
        if value and value not in VALID_FIDELITY:
            diagnostics.append(
                Diagnostic(
                    rule_id="fidelity_valid",
                    severity=DiagnosticSeverity.WARNING,
                    message=f"{rule_owner} fidelity '{value}' is not a recognized mode",
                    line=line,
                    node_id=node_id,
                    edge=edge,
                )
            )

    check_attr(
        "graph",
        graph.graph_attrs.get("default_fidelity"),
        graph.graph_attrs.get("default_fidelity").line if graph.graph_attrs.get("default_fidelity") else 0,
    )

    for node in graph.nodes.values():
        attr = node.attrs.get("fidelity")
        check_attr(f"node '{node.node_id}'", attr, attr.line if attr else node.line, node_id=node.node_id)

    for edge in graph.edges:
        attr = edge.attrs.get("fidelity")
        check_attr(
            f"edge {edge.source}->{edge.target}",
            attr,
            attr.line if attr else edge.line,
            edge=(edge.source, edge.target),
        )

    return diagnostics


def _validate_stylesheet(graph: DotGraph) -> List[Diagnostic]:
    attr = graph.graph_attrs.get("model_stylesheet")
    if not attr:
        return []

    stylesheet = attr.value
    if not isinstance(stylesheet, str):
        return [
            Diagnostic(
                rule_id="stylesheet_syntax",
                severity=DiagnosticSeverity.ERROR,
                message="model_stylesheet must be a string",
                line=attr.line,
            )
        ]

    return _lint_stylesheet_syntax(stylesheet, attr.line)


def _lint_stylesheet_syntax(stylesheet: str, line: int) -> List[Diagnostic]:
    diagnostics: List[Diagnostic] = []
    text = stylesheet.strip()
    if text == "":
        return diagnostics

    # Minimal structural parse: selector { key: value; ... }
    idx = 0
    n = len(text)
    while idx < n:
        while idx < n and text[idx].isspace():
            idx += 1
        if idx >= n:
            break

        brace = text.find("{", idx)
        if brace == -1:
            diagnostics.append(
                Diagnostic(
                    rule_id="stylesheet_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message="stylesheet selector must be followed by '{'",
                    line=line,
                )
            )
            break

        selector = text[idx:brace].strip()
        if selector == "":
            diagnostics.append(
                Diagnostic(
                    rule_id="stylesheet_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message="stylesheet selector cannot be empty",
                    line=line,
                )
            )
        elif selector != "*" and not (
            (selector.startswith(".") and _STYLESHEET_CLASS_RE.fullmatch(selector[1:] or ""))
            or (selector.startswith("#") and _STYLESHEET_ID_RE.fullmatch(selector[1:] or ""))
        ):
            diagnostics.append(
                Diagnostic(
                    rule_id="stylesheet_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message=(
                        f"invalid stylesheet selector '{selector}', must be '*', '.class', or '#node_id'"
                    ),
                    line=line,
                )
            )

        close = text.find("}", brace + 1)
        if close == -1:
            diagnostics.append(
                Diagnostic(
                    rule_id="stylesheet_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message="stylesheet block is missing closing '}'",
                    line=line,
                )
            )
            break

        body = text[brace + 1 : close].strip()
        if body == "":
            diagnostics.append(
                Diagnostic(
                    rule_id="stylesheet_syntax",
                    severity=DiagnosticSeverity.ERROR,
                    message="stylesheet rule block cannot be empty",
                    line=line,
                )
            )
        else:
            statements = [s.strip() for s in body.split(";") if s.strip()]
            for stmt in statements:
                if ":" not in stmt:
                    diagnostics.append(
                        Diagnostic(
                            rule_id="stylesheet_syntax",
                            severity=DiagnosticSeverity.ERROR,
                            message=f"stylesheet statement '{stmt}' must contain ':'",
                            line=line,
                        )
                    )
                    continue

                key, raw_value = stmt.split(":", 1)
                key = key.strip()
                if key not in _STYLESHEET_ALLOWED_PROPERTIES:
                    diagnostics.append(
                        Diagnostic(
                            rule_id="stylesheet_syntax",
                            severity=DiagnosticSeverity.ERROR,
                            message=(
                                f"unsupported stylesheet property '{key}', expected one of "
                                "llm_model, llm_provider, reasoning_effort"
                            ),
                            line=line,
                        )
                    )
                    continue

                value = raw_value.strip().strip('"')
                if key == "reasoning_effort" and value not in {"low", "medium", "high"}:
                    diagnostics.append(
                        Diagnostic(
                            rule_id="stylesheet_syntax",
                            severity=DiagnosticSeverity.ERROR,
                            message="reasoning_effort must be one of: low, medium, high",
                            line=line,
                        )
                    )

        idx = close + 1

    return diagnostics
