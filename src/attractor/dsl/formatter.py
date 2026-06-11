from __future__ import annotations

from .models import DotAttribute, DotEdge, DotGraph, DotValueType, Duration
from .parser import parse_dot


def canonicalize_dot(source: str) -> str:
    return format_dot(parse_dot(source))


def canonicalize_readable_dot(source: str) -> str:
    return format_readable_dot(parse_dot(source))


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


def format_readable_dot(graph: DotGraph) -> str:
    lines: list[str] = [f"digraph {graph.graph_id} {{"]

    if graph.graph_attrs:
        lines.append(f"  graph [{_format_attrs(graph.graph_attrs)}];")
    if graph.defaults.node:
        lines.append(f"  node [{_format_attrs(graph.defaults.node)}];")
    if graph.defaults.edge:
        lines.append(f"  edge [{_format_attrs(graph.defaults.edge)}];")

    outgoing_edges: dict[str, list[DotEdge]] = {}
    edge_ids_by_source: dict[str, list[int]] = {}
    emitted_edge_ids: set[int] = set()
    for edge_index, edge in enumerate(graph.edges):
        outgoing_edges.setdefault(edge.source, []).append(edge)
        edge_ids_by_source.setdefault(edge.source, []).append(edge_index)

    emitted_nodes: set[str] = set()

    def emit_node(node_id: str) -> None:
        if node_id in emitted_nodes or node_id not in graph.nodes:
            return
        emitted_nodes.add(node_id)
        lines.append("")
        node = graph.nodes[node_id]
        lines.append(_format_node(node_id, node.attrs))
        for edge, edge_index in zip(outgoing_edges.get(node_id, []), edge_ids_by_source.get(node_id, [])):
            lines.append(_format_edge(edge))
            emitted_edge_ids.add(edge_index)
        for edge in outgoing_edges.get(node_id, []):
            if edge.target not in emitted_nodes:
                emit_node(edge.target)

    start_node_id = _resolve_readable_start_node(graph)
    if start_node_id is not None:
        emit_node(start_node_id)

    for node_id in graph.nodes:
        emit_node(node_id)

    for edge_index, edge in enumerate(graph.edges):
        if edge_index not in emitted_edge_ids:
            lines.append(_format_edge(edge))

    lines.append("}")
    return "\n".join(lines) + "\n"


def _resolve_readable_start_node(graph: DotGraph) -> str | None:
    shape_starts: list[str] = []
    for node_id, node in graph.nodes.items():
        shape = node.attrs.get("shape")
        if shape is not None and str(shape.value) == "Mdiamond":
            shape_starts.append(node_id)
    if len(shape_starts) == 1:
        return shape_starts[0]
    for candidate in ("start", "Start"):
        if candidate in graph.nodes:
            return candidate
    return None


def _format_node(node_id: str, attrs: dict[str, DotAttribute]) -> str:
    if attrs:
        return f"  {node_id} [{_format_attrs(attrs)}];"
    return f"  {node_id};"


def _format_edge(edge: DotEdge) -> str:
    if edge.attrs:
        return f"  {edge.source} -> {edge.target} [{_format_attrs(edge.attrs)}];"
    return f"  {edge.source} -> {edge.target};"


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
