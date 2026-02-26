import pytest

from attractor.dsl import DotParseError, normalize_graph, parse_dot
from attractor.dsl.models import DotValueType, Duration


class TestDotParser:
    def test_parses_quoted_strings_with_supported_escapes(self):
        dot = r'''
        digraph EscapedStrings {
            node_a [label="line1\nline2\t\"quoted\"\\path"]
        }
        '''
        graph = parse_dot(dot)
        label = graph.nodes["node_a"].attrs["label"]

        assert label.value == 'line1\nline2\t"quoted"\\path'
        assert label.value_type == DotValueType.STRING

    def test_rejects_unescaped_newline_in_string_literal(self):
        dot = """
        digraph Bad {
            node_a [label="line1
line2"]
        }
        """
        with pytest.raises(DotParseError, match="unescaped newline in string literal"):
            parse_dot(dot)

    def test_lexes_all_supported_typed_values(self):
        dot = """
        digraph TypedValues {
            node_a [
                title="Ship it",
                max_retries=+3,
                threshold=+0.75,
                goal_gate=true,
                timeout=250ms
            ]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["node_a"].attrs

        assert attrs["title"].value == "Ship it"
        assert attrs["title"].value_type == DotValueType.STRING

        assert attrs["max_retries"].value == 3
        assert attrs["max_retries"].value_type == DotValueType.INTEGER

        assert attrs["threshold"].value == pytest.approx(0.75)
        assert attrs["threshold"].value_type == DotValueType.FLOAT

        assert attrs["goal_gate"].value is True
        assert attrs["goal_gate"].value_type == DotValueType.BOOLEAN

        timeout = attrs["timeout"]
        assert timeout.value_type == DotValueType.DURATION
        assert isinstance(timeout.value, Duration)
        assert timeout.value.raw == "250ms"

    def test_parses_duration_units_and_normalizes_representation(self):
        dot = """
        digraph DurationUnits {
            node_a [
                timeout_ms=+0250ms,
                timeout_s=0900s,
                timeout_m=15m,
                timeout_h=2h,
                timeout_d=1d
            ]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["node_a"].attrs

        expected = {
            "timeout_ms": (250, "ms", "250ms"),
            "timeout_s": (900, "s", "900s"),
            "timeout_m": (15, "m", "15m"),
            "timeout_h": (2, "h", "2h"),
            "timeout_d": (1, "d", "1d"),
        }
        for key, (number, unit, raw) in expected.items():
            attr = attrs[key]
            assert attr.value_type == DotValueType.DURATION
            assert isinstance(attr.value, Duration)
            assert attr.value.value == number
            assert attr.value.unit == unit
            assert attr.value.raw == raw

    def test_parses_only_lowercase_boolean_literals_as_boolean_type(self):
        dot = """
        digraph BooleanLiterals {
            node_a [is_ready=true, is_done=false, upper=True, mixed=False]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["node_a"].attrs

        assert attrs["is_ready"].value is True
        assert attrs["is_ready"].value_type == DotValueType.BOOLEAN
        assert attrs["is_done"].value is False
        assert attrs["is_done"].value_type == DotValueType.BOOLEAN

        assert attrs["upper"].value == "True"
        assert attrs["upper"].value_type == DotValueType.STRING
        assert attrs["mixed"].value == "False"
        assert attrs["mixed"].value_type == DotValueType.STRING

    def test_lexes_float_literals_without_leading_zero(self):
        dot = """
        digraph FloatForms {
            node_a [x=.5, y=-.5]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["node_a"].attrs

        assert attrs["x"].value == pytest.approx(0.5)
        assert attrs["x"].value_type == DotValueType.FLOAT
        assert attrs["y"].value == pytest.approx(-0.5)
        assert attrs["y"].value_type == DotValueType.FLOAT

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

    def test_accepts_mixed_statement_ordering_without_semicolons(self):
        dot = """
        digraph MixedOrdering {
            start [shape=Mdiamond]
            edge [label="next"]
            graph [goal="Ship"]
            node [timeout=15m]
            plan [prompt="Plan"]
            rankdir=LR
            done [shape=Msquare]
            start -> plan
            plan -> done
        }
        """
        graph = parse_dot(dot)

        assert graph.graph_attrs["goal"].value == "Ship"
        assert graph.graph_attrs["rankdir"].value == "LR"
        assert graph.nodes["plan"].attrs["timeout"].value.raw == "15m"
        assert [edge.source for edge in graph.edges] == ["start", "plan"]
        assert [edge.target for edge in graph.edges] == ["plan", "done"]
        assert [edge.attrs["label"].value for edge in graph.edges] == ["next", "next"]

    def test_accepts_consecutive_semicolons_between_statements(self):
        dot = """
        digraph ConsecutiveSemicolons {
            graph [goal="Ship"];;
            node [shape=box, timeout=900s];;
            edge [label="next"];;
            start [shape=Mdiamond];;
            plan [prompt="Plan"];;
            done [shape=Msquare];;
            start -> plan;;
            plan -> done;;
        }
        """
        graph = parse_dot(dot)

        assert graph.graph_attrs["goal"].value == "Ship"
        assert graph.nodes["plan"].attrs["timeout"].value.raw == "900s"
        assert len(graph.edges) == 2
        assert [edge.attrs["label"].value for edge in graph.edges] == ["next", "next"]

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

    def test_derives_subgraph_label_class_for_enclosed_nodes(self):
        dot = """
        digraph ScopedClass {
            subgraph cluster_loop {
                label="Loop A! #1"
                plan [prompt="p"]
                review [prompt="r", class="critical"]
            }
        }
        """
        graph = parse_dot(dot)

        assert graph.nodes["plan"].attrs["class"].value == "loop-a-1"
        assert graph.nodes["review"].attrs["class"].value == "critical,loop-a-1"

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
        with pytest.raises(
            DotParseError,
            match=r"invalid node id 'bad-id', must match \[A-Za-z_\]\[A-Za-z0-9_\]\*",
        ):
            parse_dot(dot)

    def test_requires_commas_between_attributes(self):
        dot = """
        digraph Bad {
            a [shape=box timeout=900s]
        }
        """
        with pytest.raises(DotParseError):
            parse_dot(dot)

    def test_rejects_trailing_comma_in_attribute_block(self):
        dot = """
        digraph Bad {
            a [shape=box,]
        }
        """
        with pytest.raises(DotParseError, match="trailing comma is not allowed in attribute blocks"):
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

    def test_reject_undirected_graph_declaration_case_insensitive(self):
        dot = """
        GRAPH Bad {
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

    def test_reject_multiple_graph_declarations_with_uppercase_keyword(self):
        dot = """
        digraph One {
            a -> b
        }
        DIGRAPH Two {
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

    def test_reject_strict_graph_modifier_case_insensitive(self):
        dot = """
        STRICT digraph G {
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

    def test_strips_comments_before_parsing_signed_numeric_literals(self):
        dot = """
        digraph G {
            a [max_retries=-/* keep */3, threshold=./* keep */5]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["a"].attrs

        assert attrs["max_retries"].value == -3
        assert attrs["max_retries"].value_type == DotValueType.INTEGER
        assert attrs["threshold"].value == pytest.approx(0.5)
        assert attrs["threshold"].value_type == DotValueType.FLOAT

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

    def test_chained_edge_trailing_typed_attrs_are_cloned_per_edge(self):
        dot = """
        digraph G {
            a -> b -> c [timeout=5s]
        }
        """
        graph = parse_dot(dot)

        first_timeout = graph.edges[0].attrs["timeout"].value
        second_timeout = graph.edges[1].attrs["timeout"].value

        assert isinstance(first_timeout, Duration)
        assert isinstance(second_timeout, Duration)
        assert first_timeout == second_timeout
        assert first_timeout is not second_timeout

    def test_chained_edge_normalization_is_equivalent_to_expanded_edges(self):
        chained = """
        digraph G {
            edge [weight=2]
            a -> b -> c [label="next", timeout=5s]
        }
        """
        expanded = """
        digraph G {
            edge [weight=2]
            a -> b [label="next", timeout=5s]
            b -> c [label="next", timeout=5s]
        }
        """

        normalized_chained = normalize_graph(parse_dot(chained))
        normalized_expanded = normalize_graph(parse_dot(expanded))

        assert normalized_chained == normalized_expanded
