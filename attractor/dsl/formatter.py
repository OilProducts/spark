from __future__ import annotations

from .models import DotAttribute, DotEdge, DotGraph, DotValueType, Duration
from .parser import parse_dot


def canonicalize_dot(source: str) -> str:
    return format_dot(parse_dot(source))


def format_dot(graph: DotGraph) -> str:
    lines: list[str] = [f"digraph {graph.graph_id} {{"]

    if graph.graph_attrs:
        lines.append(f"  graph [{_format_attrs(graph.graph_attrs)}];")

    for node_id in sorted(graph.nodes):
        node = graph.nodes[node_id]
        if node.attrs:
            lines.append(f"  {node_id} [{_format_attrs(node.attrs)}];")
        else:
            lines.append(f"  {node_id};")

    for edge in sorted(graph.edges, key=_edge_sort_key):
        if edge.attrs:
            lines.append(f"  {edge.source} -> {edge.target} [{_format_attrs(edge.attrs)}];")
        else:
            lines.append(f"  {edge.source} -> {edge.target};")

    lines.append("}")
    return "\n".join(lines) + "\n"


def _edge_sort_key(edge: DotEdge) -> tuple[str, str, str]:
    attrs = _format_attrs(edge.attrs) if edge.attrs else ""
    return edge.source, edge.target, attrs


def _format_attrs(attrs: dict[str, DotAttribute]) -> str:
    entries: list[str] = []
    for key in sorted(attrs):
        attr = attrs[key]
        entries.append(f"{key}={_format_value(attr)}")
    return ", ".join(entries)


def _format_value(attr: DotAttribute) -> str:
    value = attr.value

    if attr.value_type == DotValueType.STRING:
        return _quote_dot_string(str(value))
    if attr.value_type == DotValueType.INTEGER:
        return str(int(value))
    if attr.value_type == DotValueType.FLOAT:
        return repr(float(value))
    if attr.value_type == DotValueType.BOOLEAN:
        return "true" if bool(value) else "false"
    if attr.value_type == DotValueType.DURATION:
        if isinstance(value, Duration):
            return f"{value.value}{value.unit}"
        return str(value)

    return _quote_dot_string(str(value))


def _quote_dot_string(value: str) -> str:
    escaped: list[str] = []
    for ch in value:
        if ch == "\\":
            escaped.append("\\\\")
        elif ch == '"':
            escaped.append('\\"')
        elif ch == "\n":
            escaped.append("\\n")
        elif ch == "\t":
            escaped.append("\\t")
        else:
            escaped.append(ch)
    return '"' + "".join(escaped) + '"'
