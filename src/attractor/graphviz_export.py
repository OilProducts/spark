from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import subprocess
from typing import Optional

from attractor.dsl import DotParseError, parse_dot
from attractor.dsl.models import DotAttribute, DotEdge, DotGraph, DotValueType, Duration


@dataclass(frozen=True)
class GraphvizArtifactExport:
    source_path: Path
    dot_path: Path
    rendered_path: Optional[Path]
    error: str = ""


def export_graphviz_artifact(dot_source: str, run_root: Path) -> GraphvizArtifactExport:
    graph_dir = run_root / "artifacts" / "graphviz"
    graph_dir.mkdir(parents=True, exist_ok=True)

    source_path = graph_dir / "pipeline-source.dot"
    source_path.write_text(dot_source, encoding="utf-8")

    dot_path = graph_dir / "pipeline.dot"
    dot_path.write_text(dot_source, encoding="utf-8")

    rendered_path = graph_dir / "pipeline.svg"
    render_error = _render_graphviz(dot_path, rendered_path)
    if not render_error:
        return GraphvizArtifactExport(source_path=source_path, dot_path=dot_path, rendered_path=rendered_path)

    fallback_source = _build_graphviz_preview_source(dot_source)
    if fallback_source and fallback_source != dot_source:
        dot_path.write_text(fallback_source, encoding="utf-8")
        fallback_error = _render_graphviz(dot_path, rendered_path)
        if not fallback_error:
            return GraphvizArtifactExport(source_path=source_path, dot_path=dot_path, rendered_path=rendered_path)
        dot_path.write_text(dot_source, encoding="utf-8")
        render_error = fallback_error

    return GraphvizArtifactExport(
        source_path=source_path,
        dot_path=dot_path,
        rendered_path=None,
        error=render_error,
    )


def _render_graphviz(dot_path: Path, rendered_path: Path) -> str:
    try:
        subprocess.run(
            ["dot", "-Tsvg", str(dot_path), "-o", str(rendered_path)],
            check=True,
            capture_output=True,
            text=True,
        )
        return ""
    except FileNotFoundError:
        return "Graphviz 'dot' binary not found"
    except subprocess.CalledProcessError as exc:
        stderr = (exc.stderr or "").strip()
        message = stderr or str(exc)
        return f"Graphviz render failed: {message}"


def _build_graphviz_preview_source(dot_source: str) -> str | None:
    try:
        graph = parse_dot(dot_source)
    except DotParseError:
        return None
    return _format_graphviz_preview(graph)


def _format_graphviz_preview(graph: DotGraph) -> str:
    lines: list[str] = [f"digraph {graph.graph_id} {{"]

    if graph.graph_attrs:
        lines.append(f"  graph [{_format_graphviz_attrs(graph.graph_attrs)}];")

    for node_id in sorted(graph.nodes):
        node = graph.nodes[node_id]
        if node.attrs:
            lines.append(f"  {node_id} [{_format_graphviz_attrs(node.attrs)}];")
        else:
            lines.append(f"  {node_id};")

    for edge in sorted(graph.edges, key=_edge_sort_key):
        if edge.attrs:
            lines.append(f"  {edge.source} -> {edge.target} [{_format_graphviz_attrs(edge.attrs)}];")
        else:
            lines.append(f"  {edge.source} -> {edge.target};")

    lines.append("}")
    return "\n".join(lines) + "\n"


def _edge_sort_key(edge: DotEdge) -> tuple[str, str, str]:
    attrs = _format_graphviz_attrs(edge.attrs) if edge.attrs else ""
    return edge.source, edge.target, attrs


def _format_graphviz_attrs(attrs: dict[str, DotAttribute]) -> str:
    entries: list[str] = []
    for key in sorted(attrs):
        attr = attrs[key]
        entries.append(f'{_quote_dot_string(key)}={_format_graphviz_value(attr)}')
    return ", ".join(entries)


def _format_graphviz_value(attr: DotAttribute) -> str:
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
            return _quote_dot_string(value.raw)
        return _quote_dot_string(str(value))

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
