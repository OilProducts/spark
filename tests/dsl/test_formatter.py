from attractor.dsl import parse_dot
from attractor.dsl.formatter import canonicalize_dot, format_dot, format_readable_dot


def _line_index(lines: list[str], text: str) -> int:
    return lines.index(text)


def test_format_readable_dot_places_start_before_downstream_nodes() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  done [shape=Msquare];
  task [shape=box];
  start [shape=Mdiamond];
  task -> done;
  start -> task;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    assert _line_index(lines, '  start [shape="Mdiamond"];') < _line_index(lines, '  task [shape="box"];')
    assert _line_index(lines, '  task [shape="box"];') < _line_index(lines, '  done [shape="Msquare"];')


def test_format_readable_dot_precedes_each_node_with_blank_line() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    for node_line in (
        '  start [shape="Mdiamond"];',
        '  task [shape="box"];',
        '  done [shape="Msquare"];',
    ):
        assert lines[_line_index(lines, node_line) - 1] == ""


def test_format_readable_dot_emits_outgoing_edges_after_source_node() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    assert lines[_line_index(lines, '  start [shape="Mdiamond"];') + 1] == "  start -> task;"
    assert lines[_line_index(lines, '  task [shape="box"];') + 1] == "  task -> done;"


def test_format_readable_dot_preserves_branch_edge_order() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  start [shape=Mdiamond];
  first [shape=box];
  second [shape=box];
  done [shape=Msquare];
  start -> second [label="Second"];
  start -> first [label="First"];
  second -> done;
  first -> done;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    assert _line_index(lines, '  start -> second [label="Second"];') < _line_index(
        lines, '  start -> first [label="First"];'
    )
    assert _line_index(lines, '  second [shape="box"];') < _line_index(lines, '  first [shape="box"];')


def test_format_readable_dot_emits_loop_edge_without_duplicating_node() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> task [label="Retry"];
  task -> done;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    assert lines.count('  task [shape="box"];') == 1
    assert "  task -> task [label=\"Retry\"];" in lines


def test_format_readable_dot_appends_unreachable_nodes_in_parse_order() -> None:
    graph = parse_dot(
        """
digraph Workflow {
  orphan_b [shape=box];
  start [shape=Mdiamond];
  done [shape=Msquare];
  orphan_a [shape=box];
  start -> done;
  orphan_b -> orphan_a;
}
"""
    )

    lines = format_readable_dot(graph).splitlines()

    assert _line_index(lines, '  done [shape="Msquare"];') < _line_index(lines, '  orphan_b [shape="box"];')
    assert _line_index(lines, '  orphan_b [shape="box"];') < _line_index(lines, '  orphan_a [shape="box"];')
    assert lines[_line_index(lines, '  orphan_b [shape="box"];') + 1] == "  orphan_b -> orphan_a;"


def test_format_dot_remains_canonical_and_stable() -> None:
    dot = """
digraph Workflow {
  b [z=1, a=2];
  graph [label="Label", goal="Goal"];
  b -> a [weight=0, label="next"];
  a [shape=box];
  a -> b;
}
"""
    graph = parse_dot(dot)

    assert (
        format_dot(graph)
        == """digraph Workflow {
  graph [goal="Goal", label="Label"];
  a [shape="box"];
  b [a=2, z=1];
  a -> b;
  b -> a [label="next", weight=0];
}
"""
    )


def test_canonicalize_dot_is_idempotent() -> None:
    source = """
digraph W {
  start [shape=Mdiamond, label="Start"];
  end [shape=Msquare, label="End"];
  graph [label="L", goal="G"];
  start -> end;
}
"""
    canonical = canonicalize_dot(source)

    assert canonicalize_dot(canonical) == canonical
