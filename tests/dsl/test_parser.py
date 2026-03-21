from pathlib import Path
import re

import pytest

from attractor.dsl import DotParseError, normalize_graph, parse_dot
from attractor.dsl.models import DotValueType, Duration


SIMPLE_LINEAR_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "simple_linear_workflow.dot"
HUMAN_GATE_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "human_gate_workflow.dot"


class TestDotParser:
    def test_parses_simple_linear_workflow_fixture(self):
        graph = parse_dot(SIMPLE_LINEAR_FIXTURE.read_text(encoding="utf-8"))

        assert graph.graph_id == "Simple"
        assert graph.graph_attrs["goal"].value == "Run tests and report"
        assert list(graph.nodes.keys()) == ["start", "exit", "run_tests", "report"]
        assert [(edge.source, edge.target) for edge in graph.edges] == [
            ("start", "run_tests"),
            ("run_tests", "report"),
            ("report", "exit"),
        ]

    def test_parses_human_gate_workflow_fixture_with_labeled_options(self):
        graph = parse_dot(HUMAN_GATE_FIXTURE.read_text(encoding="utf-8"))

        assert graph.graph_id == "Review"
        assert list(graph.nodes.keys()) == ["start", "exit", "review_gate", "ship_it", "fixes"]
        assert graph.nodes["review_gate"].attrs["type"].value == "wait.human"
        assert [(edge.source, edge.target) for edge in graph.edges] == [
            ("start", "review_gate"),
            ("review_gate", "ship_it"),
            ("review_gate", "fixes"),
            ("ship_it", "exit"),
            ("fixes", "review_gate"),
        ]
        edge_labels = {
            (edge.source, edge.target): edge.attrs["label"].value
            for edge in graph.edges
            if "label" in edge.attrs
        }
        assert edge_labels[("review_gate", "ship_it")] == "[A] Approve"
        assert edge_labels[("review_gate", "fixes")] == "[F] Fix"

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

    def test_retains_subgraph_and_default_scope_metadata_item_11_1_02(self):
        dot = """
        digraph ScopedMetadata {
            node [timeout=5m]
            edge [weight=2]

            subgraph cluster_loop {
                graph [label="Loop A", ui_extension.scope="loop"]
                node [thread_id="loop-a"]
                edge [weight=9]
                plan [prompt="p"]
                review [prompt="r", custom.node_behavior="retain"]

                subgraph cluster_inner {
                    node [timeout=45s]
                    audit [prompt="a"]
                    review -> audit [custom.edge_hint="check"]
                }
            }

            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> plan
            audit -> done
        }
        """
        graph = parse_dot(dot)

        assert graph.defaults.node["timeout"].value.raw == "5m"
        assert graph.defaults.edge["weight"].value == 2
        assert len(graph.subgraphs) == 1

        loop_scope = graph.subgraphs[0]
        assert loop_scope.id == "cluster_loop"
        assert loop_scope.attrs["label"].value == "Loop A"
        assert loop_scope.attrs["ui_extension.scope"].value == "loop"
        assert set(loop_scope.node_ids) == {"plan", "review", "audit"}
        assert loop_scope.defaults.node["thread_id"].value == "loop-a"
        assert loop_scope.defaults.edge["weight"].value == 9
        assert len(loop_scope.subgraphs) == 1

        inner_scope = loop_scope.subgraphs[0]
        assert inner_scope.id == "cluster_inner"
        assert set(inner_scope.node_ids) == {"audit"}
        assert inner_scope.defaults.node["timeout"].value.raw == "45s"
        assert inner_scope.defaults.edge["weight"].value == 9

    def test_node_defaults_apply_per_subsequent_declaration_without_aliasing(self):
        dot = """
        digraph ScopedDefaults {
            node [llm_model="gpt-5-mini"]
            first [prompt="first"]

            node [llm_model="gpt-5"]
            second [prompt="second"]
            third [prompt="third"]
        }
        """
        graph = parse_dot(dot)

        assert graph.nodes["first"].attrs["llm_model"].value == "gpt-5-mini"
        assert graph.nodes["second"].attrs["llm_model"].value == "gpt-5"
        assert graph.nodes["third"].attrs["llm_model"].value == "gpt-5"

        graph.nodes["second"].attrs["llm_model"].value = "mutated"
        assert graph.nodes["third"].attrs["llm_model"].value == "gpt-5"

    def test_parses_unquoted_bare_values_with_hyphen_dot_and_colon(self):
        dot = """
        digraph BareValues {
            task [shape=box, llm_model=gpt-5.2, fidelity=summary:high]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["task"].attrs

        assert attrs["llm_model"].value == "gpt-5.2"
        assert attrs["fidelity"].value == "summary:high"

    def test_explicit_node_attrs_override_later_defaults_when_redeclared(self):
        dot = """
        digraph ExplicitWins {
            node [timeout=15m]
            plan [prompt="p", timeout=30s]

            node [timeout=1h]
            plan
        }
        """
        graph = parse_dot(dot)

        assert graph.nodes["plan"].attrs["timeout"].value.raw == "30s"

    def test_explicit_edge_attrs_override_edge_defaults(self):
        dot = """
        digraph EdgeExplicitWins {
            edge [label="default", weight=1]
            start -> plan [label="explicit", weight=9]
        }
        """
        graph = parse_dot(dot)
        edge = graph.edges[0]

        assert edge.attrs["label"].value == "explicit"
        assert edge.attrs["weight"].value == 9

    def test_edge_defaults_are_scoped_and_apply_to_subsequent_edge_declarations(self):
        dot = """
        digraph ScopedEdgeDefaults {
            edge [label="outer", weight=1]
            a -> b

            subgraph cluster_inner {
                edge [weight=9]
                b -> c
                c -> d
            }

            d -> e
        }
        """
        graph = parse_dot(dot)

        edge_attrs = [{k: v.value for k, v in edge.attrs.items()} for edge in graph.edges]
        assert edge_attrs == [
            {"label": "outer", "weight": 1},
            {"label": "outer", "weight": 9},
            {"label": "outer", "weight": 9},
            {"label": "outer", "weight": 1},
        ]

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

    def test_normalizes_comma_separated_class_attribute_values(self):
        dot = """
        digraph ClassList {
            review [class="  code , critical,code ,, lint  "]
        }
        """
        graph = parse_dot(dot)

        assert graph.nodes["review"].attrs["class"].value == "code,critical,lint"
        assert graph.nodes["review"].attrs["class"].value_type == DotValueType.STRING

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

    def test_parses_all_appendix_a_graph_attribute_keys(self):
        dot = """
        digraph GraphAttrs {
            graph [
                goal="Ship release",
                label="Release Flow",
                model_stylesheet="* { llm_model: gpt-5; }",
                default_max_retry=7,
                default_fidelity="summary:high",
                retry_target="implement",
                fallback_retry_target="plan",
                stack.child_dotfile="child.dot",
                stack.child_workdir="/tmp/child",
                tool.hooks.pre="echo pre",
                tool.hooks.post="echo post"
            ]
            start [shape=Mdiamond]
            exit [shape=Msquare]
            start -> exit
        }
        """
        graph = parse_dot(dot)

        assert set(graph.graph_attrs) == {
            "goal",
            "label",
            "model_stylesheet",
            "default_max_retry",
            "default_fidelity",
            "retry_target",
            "fallback_retry_target",
            "stack.child_dotfile",
            "stack.child_workdir",
            "tool.hooks.pre",
            "tool.hooks.post",
        }
        assert graph.graph_attrs["default_max_retry"].value == 7
        assert graph.graph_attrs["default_max_retry"].value_type == DotValueType.INTEGER
        assert graph.graph_attrs["stack.child_dotfile"].value == "child.dot"
        assert graph.graph_attrs["tool.hooks.pre"].value == "echo pre"
        assert graph.graph_attrs["tool.hooks.post"].value == "echo post"

    def test_parses_all_appendix_a_node_attribute_keys(self):
        dot = """
        digraph NodeAttrs {
            stage [
                label="Review Stage",
                shape=box,
                type=wait.human,
                prompt="Review changes",
                max_retries=3,
                goal_gate=true,
                retry_target=fix_stage,
                fallback_retry_target=plan_stage,
                fidelity="summary:high",
                thread_id="review-thread",
                class="review,critical",
                timeout=45s,
                llm_model="gpt-5.2",
                llm_provider=openai,
                reasoning_effort=medium,
                auto_status=true,
                allow_partial=false
            ]
        }
        """
        graph = parse_dot(dot)
        attrs = graph.nodes["stage"].attrs

        assert set(attrs) == {
            "label",
            "shape",
            "type",
            "prompt",
            "max_retries",
            "goal_gate",
            "retry_target",
            "fallback_retry_target",
            "fidelity",
            "thread_id",
            "class",
            "timeout",
            "llm_model",
            "llm_provider",
            "reasoning_effort",
            "auto_status",
            "allow_partial",
        }
        assert graph.nodes["stage"].explicit_attr_keys == set(attrs)
        assert attrs["max_retries"].value_type == DotValueType.INTEGER
        assert attrs["goal_gate"].value_type == DotValueType.BOOLEAN
        assert attrs["auto_status"].value_type == DotValueType.BOOLEAN
        assert attrs["allow_partial"].value_type == DotValueType.BOOLEAN
        assert attrs["timeout"].value_type == DotValueType.DURATION

    def test_rejects_malformed_graph_assignment_key(self):
        dot = """
        digraph Bad {
            tool_hooks..pre="echo pre"
        }
        """
        with pytest.raises(DotParseError, match="invalid attribute key"):
            parse_dot(dot)

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


class TestDotParserDefinitionOfDone11_1:
    def test_accepts_supported_dot_subset_with_graph_node_and_edge_blocks(self):
        graph = parse_dot(
            """
            DIGRAPH G {
                graph [goal="Ship"]
                node [shape=box]
                edge [weight=2]
                start [shape=Mdiamond]
                work [prompt="Do it"]
                done [shape=Msquare]
                start -> work -> done
            }
            """
        )

        assert graph.graph_id == "G"
        assert graph.graph_attrs["goal"].value == "Ship"
        assert graph.nodes["work"].attrs["shape"].value == "box"
        assert [edge.attrs["weight"].value for edge in graph.edges] == [2, 2]

    def test_extracts_goal_label_and_model_stylesheet_graph_attributes(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship release", label="Release Flow", model_stylesheet="* { llm_model: gpt-5; }"]
            }
            """
        )

        assert graph.graph_attrs["goal"].value == "Ship release"
        assert graph.graph_attrs["label"].value == "Release Flow"
        assert graph.graph_attrs["model_stylesheet"].value == "* { llm_model: gpt-5; }"

    def test_parses_multiline_node_attribute_blocks(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    prompt="Write summary",
                    timeout=45s,
                    goal_gate=true
                ]
            }
            """
        )

        attrs = graph.nodes["task"].attrs
        assert attrs["prompt"].value == "Write summary"
        assert attrs["timeout"].value.raw == "45s"
        assert attrs["goal_gate"].value is True

    def test_parses_all_supported_edge_attributes(self):
        graph = parse_dot(
            """
            digraph G {
                review -> fix [
                    label="retry",
                    condition="outcome=fail",
                    weight=7,
                    fidelity=summary:low,
                    thread_id="review-thread",
                    loop_restart=true
                ]
            }
            """
        )
        edge = graph.edges[0]

        assert edge.attrs["label"].value == "retry"
        assert edge.attrs["condition"].value == "outcome=fail"
        assert edge.attrs["weight"].value == 7
        assert edge.attrs["fidelity"].value == "summary:low"
        assert edge.attrs["thread_id"].value == "review-thread"
        assert edge.attrs["loop_restart"].value is True

    def test_expands_chained_edges_into_pairwise_edges(self):
        graph = parse_dot(
            """
            digraph G {
                A -> B -> C
            }
            """
        )

        assert [(edge.source, edge.target) for edge in graph.edges] == [("A", "B"), ("B", "C")]

    def test_node_and_edge_defaults_apply_to_subsequent_declarations(self):
        graph = parse_dot(
            """
            digraph G {
                node [timeout=15m]
                edge [weight=3]
                alpha [prompt="a"]
                beta [prompt="b"]
                alpha -> beta
            }
            """
        )

        assert graph.nodes["alpha"].attrs["timeout"].value.raw == "15m"
        assert graph.nodes["beta"].attrs["timeout"].value.raw == "15m"
        assert graph.edges[0].attrs["weight"].value == 3

    def test_subgraph_contents_are_flattened_into_top_level_graph(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                subgraph cluster_loop {
                    plan [prompt="Plan"]
                    review [prompt="Review"]
                    plan -> review
                }
                start -> plan
                review -> done
            }
            """
        )

        assert set(graph.nodes.keys()) == {"start", "done", "plan", "review"}
        assert {(edge.source, edge.target) for edge in graph.edges} == {
            ("plan", "review"),
            ("start", "plan"),
            ("review", "done"),
        }

    def test_node_class_attribute_merges_stylesheet_attributes(self):
        from attractor.transforms.stylesheet import ModelStylesheetTransform

        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".fast { llm_model: gpt-5-mini; llm_provider: openai; reasoning_effort: low; }"]
                start [shape=Mdiamond]
                task [shape=box, class="fast"]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        ModelStylesheetTransform().apply(graph)
        task_attrs = graph.nodes["task"].attrs
        assert task_attrs["llm_model"].value == "gpt-5-mini"
        assert task_attrs["llm_provider"].value == "openai"
        assert task_attrs["reasoning_effort"].value == "low"

    def test_accepts_quoted_and_unquoted_attribute_values(self):
        graph = parse_dot(
            """
            digraph G {
                task [quoted="Ship", unquoted=Ship]
            }
            """
        )

        assert graph.nodes["task"].attrs["quoted"].value == "Ship"
        assert graph.nodes["task"].attrs["quoted"].value_type == DotValueType.STRING
        assert graph.nodes["task"].attrs["unquoted"].value == "Ship"
        assert graph.nodes["task"].attrs["unquoted"].value_type == DotValueType.STRING

    def test_strips_line_and_block_comments_before_parsing(self):
        graph = parse_dot(
            """
            digraph G {
                // node declaration
                start [shape=Mdiamond]
                /* comment with invalid syntax that must be ignored:
                   start -- bad
                */
                done [shape=Msquare]
                start -> done
            }
            """
        )

        assert set(graph.nodes.keys()) == {"start", "done"}
        assert [(edge.source, edge.target) for edge in graph.edges] == [("start", "done")]


class TestDotParserUnsupportedGrammarRegression:
    @pytest.mark.parametrize(
        ("dot", "message"),
        [
            (
                """
                strict digraph G {
                    a -> b
                }
                """,
                "strict modifier is not supported",
            ),
            (
                """
                graph G {
                    a -> b
                }
                """,
                "undirected graph declarations are not supported",
            ),
            (
                """
                digraph G {
                    a -- b
                }
                """,
                "undirected edges ('--') are not supported",
            ),
            (
                """
                digraph G {
                    a [label=<b>Bold</b>]
                }
                """,
                "HTML-like labels are not supported",
            ),
            (
                """
                digraph G {
                    a:out -> b
                }
                """,
                "port and compass point syntax is not supported",
            ),
        ],
    )
    def test_unsupported_grammar_regression_inputs_are_rejected(self, dot: str, message: str):
        with pytest.raises(DotParseError, match=re.escape(message)):
            parse_dot(dot)
