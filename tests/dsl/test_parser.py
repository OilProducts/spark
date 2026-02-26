import pytest

from attractor.dsl import DotParseError, parse_dot
from attractor.dsl.models import DotValueType, Duration


class TestDotParser:
    def test_parse_basic_graph_with_typed_attrs_and_chained_edges(self):
        dot = """
        // graph comment
        digraph Demo {
            graph [goal="Ship", default_max_retry=5]
            node [shape=box, timeout=900s]
            edge [weight=2]

            start [shape=Mdiamond, label="Start"]
            plan [prompt="Plan for $goal"]
            done [shape=Msquare]

            start -> plan -> done [label="next"]
        }
        """
        graph = parse_dot(dot)

        assert graph.graph_id == "Demo"
        assert "goal" in graph.graph_attrs
        assert graph.graph_attrs["goal"].value == "Ship"
        assert graph.graph_attrs["default_max_retry"].value == 5

        assert len(graph.edges) == 2
        assert graph.edges[0].source == "start"
        assert graph.edges[0].target == "plan"
        assert graph.edges[0].attrs["weight"].value == 2
        assert graph.edges[0].attrs["label"].value == "next"

        plan_timeout = graph.nodes["plan"].attrs["timeout"]
        assert plan_timeout.value_type == DotValueType.DURATION
        assert isinstance(plan_timeout.value, Duration)
        assert plan_timeout.value.raw == "900s"

    def test_parse_subgraph_scope_defaults(self):
        dot = """
        digraph Scoped {
            start [shape=Mdiamond]
            done [shape=Msquare]

            subgraph cluster_loop {
                node [thread_id="loop-a", timeout=15m]
                plan [prompt="p"]
            }

            start -> plan -> done
        }
        """
        graph = parse_dot(dot)
        plan = graph.nodes["plan"]
        assert plan.attrs["thread_id"].value == "loop-a"
        assert plan.attrs["timeout"].value.raw == "15m"

    def test_subgraph_attr_decl_does_not_override_graph_attrs(self):
        dot = """
        digraph Scoped {
            label="Top Level"

            subgraph cluster_loop {
                label="Loop A"
                node [thread_id="loop-a"]
                plan [prompt="p"]
            }

            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> plan -> done
        }
        """
        graph = parse_dot(dot)
        assert graph.graph_attrs["label"].value == "Top Level"

    def test_reject_undirected_edges(self):
        dot = """
        digraph Bad {
            a -- b
        }
        """
        with pytest.raises(DotParseError):
            parse_dot(dot)

    def test_reject_invalid_node_id(self):
        dot = """
        digraph Bad {
            bad-id [prompt="x"]
        }
        """
        with pytest.raises(DotParseError):
            parse_dot(dot)

    def test_requires_commas_between_attributes(self):
        dot = """
        digraph Bad {
            a [shape=box timeout=900s]
        }
        """
        with pytest.raises(DotParseError):
            parse_dot(dot)

    def test_parse_qualified_attribute_keys(self):
        dot = """
        digraph Qualified {
            a [model.provider="openai", model.reasoning.effort="high"]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["a"].attrs

        assert attrs["model.provider"].value == "openai"
        assert attrs["model.reasoning.effort"].value == "high"

    def test_reject_malformed_qualified_attribute_key(self):
        dot = """
        digraph Bad {
            a [model..provider="openai"]
        }
        """
        with pytest.raises(DotParseError, match="invalid attribute key"):
            parse_dot(dot)

    def test_reject_undirected_graph_declaration(self):
        dot = """
        graph Bad {
            a -> b
        }
        """
        with pytest.raises(DotParseError, match="undirected graph declarations are not supported"):
            parse_dot(dot)

    def test_reject_multiple_graph_declarations(self):
        dot = """
        digraph One {
            a -> b
        }
        digraph Two {
            c -> d
        }
        """
        with pytest.raises(DotParseError, match="multiple graph declarations are not supported"):
            parse_dot(dot)

    def test_reject_multiple_graph_declarations_when_separated_by_semicolon(self):
        dot = """
        digraph One {
            a -> b
        };
        digraph Two {
            c -> d
        }
        """
        with pytest.raises(DotParseError, match="multiple graph declarations are not supported"):
            parse_dot(dot)

    def test_reject_strict_graph_modifier(self):
        dot = """
        strict digraph G {
            a -> b
        }
        """
        with pytest.raises(DotParseError, match="strict modifier is not supported"):
            parse_dot(dot)

    def test_reject_html_like_label_value(self):
        dot = """
        digraph G {
            a [label=<b>Bold</b>]
        }
        """
        with pytest.raises(DotParseError, match="HTML-like labels are not supported"):
            parse_dot(dot)

    def test_reject_port_or_compass_point_syntax(self):
        dot = """
        digraph G {
            a:out -> b:in
        }
        """
        with pytest.raises(DotParseError, match="port and compass point syntax is not supported"):
            parse_dot(dot)

    def test_strip_block_comments_before_parse(self):
        dot = """
        digraph G {
            /* this edge should be ignored: a -- b */
            a -> b
        }
        """
        graph = parse_dot(dot)
        assert len(graph.edges) == 1
        assert graph.edges[0].source == "a"
        assert graph.edges[0].target == "b"

    def test_chained_edge_trailing_attrs_are_copied_per_edge(self):
        dot = """
        digraph G {
            a -> b -> c [label="next"]
        }
        """
        graph = parse_dot(dot)

        assert len(graph.edges) == 2
        assert graph.edges[0].attrs["label"].value == "next"
        assert graph.edges[1].attrs["label"].value == "next"

        graph.edges[0].attrs["label"].value = "changed"
        assert graph.edges[1].attrs["label"].value == "next"
