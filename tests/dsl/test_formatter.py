from attractor.dsl import parse_dot
from attractor.dsl.formatter import canonicalize_dot, format_dot


def test_format_dot_stable_ordering() -> None:
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
