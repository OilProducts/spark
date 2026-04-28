from pathlib import Path

from attractor.dsl import parse_dot
from attractor.dsl.models import DotAttribute, DotValueType
from attractor.transforms import (
    GoalVariableTransform,
    GraphMergeTransform,
    ModelStylesheetTransform,
    TransformPipeline,
)
from attractor.transforms import AttributeDefaultsTransform


class TestTransforms:
    def test_attribute_defaults_transform_injects_required_graph_defaults_with_types(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)

        expected_graph_defaults = {
            "goal": ("", DotValueType.STRING),
            "label": ("", DotValueType.STRING),
            "model_stylesheet": ("", DotValueType.STRING),
            "default_max_retries": (0, DotValueType.INTEGER),
            "default_fidelity": ("", DotValueType.STRING),
            "tool.hooks.pre": ("", DotValueType.STRING),
            "tool.hooks.post": ("", DotValueType.STRING),
        }

        for attr_name, (expected_value, expected_type) in expected_graph_defaults.items():
            attr = graph.graph_attrs[attr_name]
            assert attr.value == expected_value
            assert attr.value_type == expected_type

        child_dotfile = graph.graph_attrs["stack.child_dotfile"]
        child_workdir = graph.graph_attrs["stack.child_workdir"]
        assert child_dotfile.value == ""
        assert child_dotfile.value_type == DotValueType.STRING
        assert isinstance(child_workdir.value, str)
        assert child_workdir.value
        assert child_workdir.value_type == DotValueType.STRING

    def test_attribute_defaults_transform_sets_missing_node_shape_and_label(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)

        task = graph.nodes["task"]
        assert task.attrs["shape"].value == "box"
        assert task.attrs["shape"].value_type == DotValueType.STRING
        assert task.attrs["label"].value == "task"
        assert task.attrs["label"].value_type == DotValueType.STRING

    def test_attribute_defaults_transform_injects_typed_node_runtime_defaults(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)

        task = graph.nodes["task"]
        expected_node_defaults = {
            "type": ("", DotValueType.STRING),
            "prompt": ("", DotValueType.STRING),
            "max_retries": (0, DotValueType.INTEGER),
            "goal_gate": (False, DotValueType.BOOLEAN),
            "retry_target": ("", DotValueType.STRING),
            "fallback_retry_target": ("", DotValueType.STRING),
            "fidelity": ("", DotValueType.STRING),
            "thread_id": ("", DotValueType.STRING),
            "class": ("", DotValueType.STRING),
            "llm_model": ("", DotValueType.STRING),
            "llm_provider": ("", DotValueType.STRING),
            "reasoning_effort": ("high", DotValueType.STRING),
            "auto_status": (False, DotValueType.BOOLEAN),
            "allow_partial": (False, DotValueType.BOOLEAN),
        }

        for attr_name, (expected_value, expected_type) in expected_node_defaults.items():
            attr = task.attrs[attr_name]
            assert attr.value == expected_value
            assert attr.value_type == expected_type

        assert task.attrs["timeout"].value is None
        assert task.attrs["timeout"].value_type == DotValueType.DURATION

    def test_attribute_defaults_transform_injects_typed_edge_defaults(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)

        edge = graph.edges[0]
        assert edge.attrs["label"].value == ""
        assert edge.attrs["label"].value_type == DotValueType.STRING
        assert edge.attrs["condition"].value == ""
        assert edge.attrs["condition"].value_type == DotValueType.STRING
        assert edge.attrs["weight"].value == 0
        assert edge.attrs["weight"].value_type == DotValueType.INTEGER
        assert edge.attrs["loop_restart"].value is False
        assert edge.attrs["loop_restart"].value_type == DotValueType.BOOLEAN

    def test_attribute_defaults_transform_preserves_explicit_values(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship", default_max_retries=9]
                start [shape=Mdiamond]
                task [
                    label="Task Label",
                    shape=hexagon,
                    max_retries=3,
                    goal_gate=true,
                    reasoning_effort="medium",
                    auto_status=true,
                    allow_partial=true
                ]
                done [shape=Msquare]
                start -> task [label="go", condition="outcome=success", weight=7, loop_restart=true]
                task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)

        assert graph.graph_attrs["goal"].value == "Ship"
        assert graph.graph_attrs["default_max_retries"].value == 9

        task = graph.nodes["task"]
        assert task.attrs["label"].value == "Task Label"
        assert task.attrs["shape"].value == "hexagon"
        assert task.attrs["max_retries"].value == 3
        assert task.attrs["goal_gate"].value is True
        assert task.attrs["reasoning_effort"].value == "medium"
        assert task.attrs["auto_status"].value is True
        assert task.attrs["allow_partial"].value is True

        edge = graph.edges[0]
        assert edge.attrs["label"].value == "go"
        assert edge.attrs["condition"].value == "outcome=success"
        assert edge.attrs["weight"].value == 7
        assert edge.attrs["loop_restart"].value is True

    def test_goal_variable_transform(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Build API"]
                start [shape=Mdiamond]
                plan [shape=box, prompt="Plan for $goal"]
                done [shape=Msquare]
                start -> plan -> done
            }
            """
        )

        GoalVariableTransform().apply(graph)
        assert graph.nodes["plan"].attrs["prompt"].value == "Plan for Build API"

    def test_goal_variable_transform_replaces_with_empty_goal(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, prompt="Plan for $goal"]
                done [shape=Msquare]
                start -> plan -> done
            }
            """
        )

        GoalVariableTransform().apply(graph)
        assert graph.nodes["plan"].attrs["prompt"].value == "Plan for "

    def test_stylesheet_specificity_and_explicit_override(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: base; llm_provider: generic; } .fast { llm_model: flash; } #review { llm_model: best; llm_provider: openai; reasoning_effort: xhigh; }"]

                start [shape=Mdiamond]
                plan [shape=box, class="fast"]
                review [shape=box, class="fast", llm_model="explicit"]
                done [shape=Msquare]
                start -> plan -> review -> done
            }
            """
        )

        ModelStylesheetTransform().apply(graph)

        # class overrides shape and universal
        assert graph.nodes["plan"].attrs["llm_model"].value == "flash"
        # explicit attribute is not overwritten
        assert graph.nodes["review"].attrs["llm_model"].value == "explicit"
        # other properties from highest-specific rule can still apply
        assert graph.nodes["review"].attrs["llm_provider"].value == "openai"
        assert graph.nodes["review"].attrs["reasoning_effort"].value == "xhigh"

    def test_stylesheet_class_selector_applies_with_comma_separated_classes_after_defaults(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".critical { llm_model: gpt-5.2; llm_provider: openai; }"]
                start [shape=Mdiamond]
                review [shape=box, class="code, critical"]
                done [shape=Msquare]
                start -> review -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["review"].attrs["llm_model"].value == "gpt-5.2"
        assert graph.nodes["review"].attrs["llm_provider"].value == "openai"

    def test_stylesheet_class_selector_matches_normalized_parsed_class_list(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".critical { llm_model: gpt-5.2; }"]
                start [shape=Mdiamond]
                review [shape=box, class=" Critical , code , CRITICAL "]
                done [shape=Msquare]
                start -> review -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["review"].attrs["llm_model"].value == "gpt-5.2"

    def test_stylesheet_shape_selector_applies_between_universal_and_class_specificity(self):
        graph = parse_dot(
            """
            digraph G {
                graph [
                    model_stylesheet="* { llm_provider: universal; } box { llm_model: shape-model; llm_provider: shape-provider; } .fast { llm_model: class-model; }"
                ]
                start [shape=Mdiamond]
                plan [shape=box, class="fast"]
                review [shape=box]
                done [shape=Msquare]
                start -> plan -> review -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["plan"].attrs["llm_model"].value == "class-model"
        assert graph.nodes["plan"].attrs["llm_provider"].value == "shape-provider"
        assert graph.nodes["review"].attrs["llm_model"].value == "shape-model"
        assert graph.nodes["review"].attrs["llm_provider"].value == "shape-provider"

    def test_stylesheet_multiple_matching_classes_use_rule_order_for_equal_specificity(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".quality { llm_model: model-quality; llm_provider: provider-quality; } .urgent { llm_model: model-urgent; }"]
                start [shape=Mdiamond]
                review [shape=box, class="quality, urgent"]
                done [shape=Msquare]
                start -> review -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Both selectors match; later equal-specificity rule wins for llm_model.
        assert graph.nodes["review"].attrs["llm_model"].value == "model-urgent"
        # Unset properties continue to use the best available earlier match.
        assert graph.nodes["review"].attrs["llm_provider"].value == "provider-quality"

    def test_stylesheet_multiple_matching_classes_still_yield_to_id_specificity(self):
        graph = parse_dot(
            """
            digraph G {
                graph [
                    model_stylesheet=".quality { llm_model: model-quality; llm_provider: provider-quality; } .urgent { llm_model: model-urgent; llm_provider: provider-urgent; } #review { llm_provider: provider-review; reasoning_effort: low; }"
                ]
                start [shape=Mdiamond]
                review [shape=box, class="quality, urgent"]
                done [shape=Msquare]
                start -> review -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["review"].attrs["llm_model"].value == "model-urgent"
        assert graph.nodes["review"].attrs["llm_provider"].value == "provider-review"
        assert graph.nodes["review"].attrs["reasoning_effort"].value == "low"

    def test_model_attr_precedence_node_then_stylesheet_then_graph_default(self):
        graph = parse_dot(
            """
            digraph G {
                graph [
                    llm_model="graph-model",
                    llm_provider="graph-provider",
                    llm_profile="graph-profile",
                    reasoning_effort="low",
                    model_stylesheet=".fast { llm_model: class-model; llm_profile: class-profile; } #review { llm_model: review-model; llm_provider: review-provider; llm_profile: review-profile; }"
                ]
                start [shape=Mdiamond]
                plan [shape=box, class="fast"]
                review [shape=box]
                explicit [shape=box, class="fast", llm_model="node-model", llm_provider="node-provider", llm_profile="node-profile", reasoning_effort="medium"]
                done [shape=Msquare]
                start -> plan -> review -> explicit -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Stylesheet wins over graph defaults.
        assert graph.nodes["plan"].attrs["llm_model"].value == "class-model"
        # Graph defaults fill unresolved attrs.
        assert graph.nodes["plan"].attrs["llm_provider"].value == "graph-provider"
        assert graph.nodes["plan"].attrs["llm_profile"].value == "class-profile"
        assert graph.nodes["plan"].attrs["reasoning_effort"].value == "low"

        # ID selector wins over class/universal and graph defaults.
        assert graph.nodes["review"].attrs["llm_model"].value == "review-model"
        assert graph.nodes["review"].attrs["llm_provider"].value == "review-provider"
        assert graph.nodes["review"].attrs["llm_profile"].value == "review-profile"
        assert graph.nodes["review"].attrs["reasoning_effort"].value == "low"

        # Explicit node attrs always win.
        assert graph.nodes["explicit"].attrs["llm_model"].value == "node-model"
        assert graph.nodes["explicit"].attrs["llm_provider"].value == "node-provider"
        assert graph.nodes["explicit"].attrs["llm_profile"].value == "node-profile"
        assert graph.nodes["explicit"].attrs["reasoning_effort"].value == "medium"

    def test_model_attr_precedence_includes_system_defaults_when_attrs_missing(self):
        graph = parse_dot(
            """
            digraph G {
                graph [
                    llm_provider="graph-provider",
                    model_stylesheet=".fast { llm_model: style-model; }"
                ]
                start [shape=Mdiamond]
                plan [shape=box, class="fast"]
                explicit [shape=box, class="fast", llm_model="node-model"]
                plain [shape=box]
                done [shape=Msquare]
                start -> plan -> explicit -> plain -> done
            }
            """
        )

        ModelStylesheetTransform().apply(graph)

        # Explicit node attrs are highest precedence.
        assert graph.nodes["explicit"].attrs["llm_model"].value == "node-model"
        # Stylesheet values beat graph defaults.
        assert graph.nodes["plan"].attrs["llm_model"].value == "style-model"
        # Graph defaults fill unresolved attrs.
        assert graph.nodes["plan"].attrs["llm_provider"].value == "graph-provider"
        assert graph.nodes["plain"].attrs["llm_provider"].value == "graph-provider"
        # System defaults fill any remaining gaps.
        assert graph.nodes["plan"].attrs["reasoning_effort"].value == "high"
        assert graph.nodes["plain"].attrs["llm_model"].value == ""
        assert graph.nodes["plain"].attrs["reasoning_effort"].value == "high"

    def test_stylesheet_overrides_node_default_model_attrs_but_not_explicit_node_attrs(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".fast { llm_model: style-model; llm_provider: style-provider; reasoning_effort: low; }"]
                node [llm_model="default-model", llm_provider="default-provider", reasoning_effort="medium"]
                start [shape=Mdiamond]
                implicit [shape=box, class="fast"]
                explicit [shape=box, class="fast", llm_model="node-model", llm_provider="node-provider", reasoning_effort="high"]
                done [shape=Msquare]
                start -> implicit -> explicit -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Node defaults are not explicit per-node attributes, so stylesheet can override.
        assert graph.nodes["implicit"].attrs["llm_model"].value == "style-model"
        assert graph.nodes["implicit"].attrs["llm_provider"].value == "style-provider"
        assert graph.nodes["implicit"].attrs["reasoning_effort"].value == "low"

        # Explicit per-node attributes still win over stylesheet values.
        assert graph.nodes["explicit"].attrs["llm_model"].value == "node-model"
        assert graph.nodes["explicit"].attrs["llm_provider"].value == "node-provider"
        assert graph.nodes["explicit"].attrs["reasoning_effort"].value == "high"

    def test_stylesheet_overrides_inherited_model_defaults_even_when_declared_on_same_line(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".fast { llm_model: style-model; llm_provider: style-provider; reasoning_effort: low; }"]
                node [llm_model="default-model", llm_provider="default-provider", reasoning_effort="medium"] implicit [shape=box, class="fast"]
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> implicit -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["implicit"].attrs["llm_model"].value == "style-model"
        assert graph.nodes["implicit"].attrs["llm_provider"].value == "style-provider"
        assert graph.nodes["implicit"].attrs["reasoning_effort"].value == "low"

    def test_stylesheet_overrides_same_line_node_defaults_for_node_without_explicit_attrs(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".fast { llm_model: style-model; llm_provider: style-provider; reasoning_effort: low; }"]
                node [llm_model="default-model", llm_provider="default-provider", reasoning_effort="medium", class="fast"] implicit
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> implicit -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["implicit"].attrs["llm_model"].value == "style-model"
        assert graph.nodes["implicit"].attrs["llm_provider"].value == "style-provider"
        assert graph.nodes["implicit"].attrs["reasoning_effort"].value == "low"

    def test_stylesheet_overrides_late_inherited_node_defaults_without_explicit_attrs(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: style-model; llm_provider: style-provider; reasoning_effort: low; }"]
                start [shape=Mdiamond]
                implicit
                node [llm_model="default-model", llm_provider="default-provider", reasoning_effort="medium"]
                implicit
                done [shape=Msquare]
                start -> implicit -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Even when inherited defaults are attached on a later redeclaration,
        # they remain overridable by stylesheet values.
        assert graph.nodes["implicit"].attrs["llm_model"].value == "style-model"
        assert graph.nodes["implicit"].attrs["llm_provider"].value == "style-provider"
        assert graph.nodes["implicit"].attrs["reasoning_effort"].value == "low"

    def test_stylesheet_parses_quoted_values_with_semicolons(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: \\"gpt;v2\\"; llm_provider: openai; }"]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["task"].attrs["llm_model"].value == "gpt;v2"
        assert graph.nodes["task"].attrs["llm_provider"].value == "openai"

    def test_stylesheet_unescapes_quoted_llm_model_value(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )
        graph.graph_attrs["model_stylesheet"] = DotAttribute(
            key="model_stylesheet",
            value='* { llm_model: "alpha\\"beta\\\\v2"; }',
            value_type=DotValueType.STRING,
            line=1,
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert graph.nodes["task"].attrs["llm_model"].value == 'alpha"beta\\v2'

    def test_stylesheet_rejects_rule_containing_unsupported_property(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: gpt-5; temperature: 0.2; llm_provider: openai; }"]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Unsupported properties invalidate the containing rule.
        assert graph.nodes["task"].attrs["llm_model"].value == ""
        assert graph.nodes["task"].attrs["llm_provider"].value == ""

    def test_stylesheet_rejects_invalid_reasoning_effort_value(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: gpt-5; reasoning_effort: ultra; }"]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Invalid reasoning_effort values must invalidate the containing rule.
        assert graph.nodes["task"].attrs["llm_model"].value == ""
        assert graph.nodes["task"].attrs["reasoning_effort"].value == "high"

    def test_stylesheet_rejects_invalid_class_selector_and_malformed_declaration(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet=".bad$class { llm_model: invalid-selector; } * { llm_model: gpt-5 llm_provider: openai; }"]
                start [shape=Mdiamond]
                task [shape=box, class="bad$class"]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Invalid selectors and malformed declarations must not apply.
        assert graph.nodes["task"].attrs["llm_model"].value == ""
        assert graph.nodes["task"].attrs["llm_provider"].value == ""

    def test_stylesheet_rejects_equals_sign_declaration_syntax(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model = gpt-5; llm_provider: openai; }"]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Declarations must use "property: value"; "=" invalidates the rule.
        assert graph.nodes["task"].attrs["llm_model"].value == ""
        assert graph.nodes["task"].attrs["llm_provider"].value == ""

    def test_stylesheet_rejects_malformed_quoted_value(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )
        graph.graph_attrs["model_stylesheet"] = DotAttribute(
            key="model_stylesheet",
            value='* { llm_model: "alpha""beta"; llm_provider: openai; }',
            value_type=DotValueType.STRING,
            line=1,
        )

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        # Malformed quoted strings must invalidate the containing rule.
        assert graph.nodes["task"].attrs["llm_model"].value == ""
        assert graph.nodes["task"].attrs["llm_provider"].value == ""

    def test_stylesheet_spec_example_fixture_parses_documented_structure(self):
        fixture_path = Path(__file__).resolve().parents[1] / "fixtures" / "stylesheet_precedence_example.dot"
        graph = parse_dot(fixture_path.read_text(encoding="utf-8"))

        assert graph.graph_id == "Pipeline"
        assert graph.graph_attrs["goal"].value == "Implement feature X"
        assert "critical_review" in graph.nodes
        assert graph.nodes["plan"].attrs["class"].value == "planning"
        assert graph.nodes["implement"].attrs["class"].value == "code"
        assert graph.nodes["critical_review"].attrs["class"].value == "code"
        assert [(edge.source, edge.target) for edge in graph.edges] == [
            ("start", "plan"),
            ("plan", "implement"),
            ("implement", "critical_review"),
            ("critical_review", "exit"),
        ]

    def test_stylesheet_spec_example_fixture_resolves_model_provider_and_reasoning(self):
        fixture_path = Path(__file__).resolve().parents[1] / "fixtures" / "stylesheet_precedence_example.dot"
        graph = parse_dot(fixture_path.read_text(encoding="utf-8"))

        AttributeDefaultsTransform().apply(graph)
        ModelStylesheetTransform().apply(graph)

        assert (
            graph.nodes["plan"].attrs["llm_model"].value,
            graph.nodes["plan"].attrs["llm_provider"].value,
            graph.nodes["plan"].attrs["reasoning_effort"].value,
        ) == ("claude-sonnet-4-5", "anthropic", "high")
        assert (
            graph.nodes["implement"].attrs["llm_model"].value,
            graph.nodes["implement"].attrs["llm_provider"].value,
            graph.nodes["implement"].attrs["reasoning_effort"].value,
        ) == ("claude-opus-4-6", "anthropic", "high")
        assert (
            graph.nodes["critical_review"].attrs["llm_model"].value,
            graph.nodes["critical_review"].attrs["llm_provider"].value,
            graph.nodes["critical_review"].attrs["reasoning_effort"].value,
        ) == ("gpt-5.2", "openai", "high")

    def test_transform_pipeline_order(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Landing Page", model_stylesheet="* { llm_model: gpt-5; }"]
                start [shape=Mdiamond]
                task [shape=box, prompt="Build $goal"]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )

        pipeline = TransformPipeline()
        pipeline.register(GoalVariableTransform())
        pipeline.register(ModelStylesheetTransform())
        graph = pipeline.apply(graph)

        assert graph.nodes["task"].attrs["prompt"].value == "Build Landing Page"
        assert graph.nodes["task"].attrs["llm_model"].value == "gpt-5"

    def test_transform_pipeline_does_not_mutate_input_graph(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Landing Page", model_stylesheet="* { llm_model: gpt-5; }"]
                start [shape=Mdiamond]
                task [shape=box, prompt="Build $goal"]
                done [shape=Msquare]
                start -> task -> done
            }
            """
        )
        original_prompt = graph.nodes["task"].attrs["prompt"].value
        assert "llm_model" not in graph.nodes["task"].attrs

        pipeline = TransformPipeline()
        pipeline.register(GoalVariableTransform())
        pipeline.register(ModelStylesheetTransform())
        transformed = pipeline.apply(graph)

        assert transformed is not graph
        assert graph.nodes["task"].attrs["prompt"].value == original_prompt
        assert "llm_model" not in graph.nodes["task"].attrs
        assert transformed.nodes["task"].attrs["prompt"].value == "Build Landing Page"
        assert transformed.nodes["task"].attrs["llm_model"].value == "gpt-5"

    def test_graph_merge_transform_merges_module_nodes_edges_and_missing_attrs(self):
        base_graph = parse_dot(
            """
            digraph Base {
                graph [goal="Ship feature"]
                start [shape=Mdiamond]
                plan [shape=box, prompt="Plan path"]
                done [shape=Msquare]
                start -> plan
            }
            """
        )
        module_graph = parse_dot(
            """
            digraph Module {
                graph [default_fidelity="summary:low"]
                plan [llm_model="gpt-5"]
                review [shape=hexagon, prompt="Review plan"]
                done [shape=Msquare]
                plan -> review
                review -> done [label="approved"]
            }
            """
        )

        merged = GraphMergeTransform(module_graph).apply(base_graph)

        # Existing graph-level attrs are preserved; missing attrs can be filled from module.
        assert merged.graph_attrs["goal"].value == "Ship feature"
        assert merged.graph_attrs["default_fidelity"].value == "summary:low"

        # Existing node attrs are preserved, while missing attrs are merged in.
        assert merged.nodes["plan"].attrs["prompt"].value == "Plan path"
        assert merged.nodes["plan"].attrs["llm_model"].value == "gpt-5"
        assert merged.nodes["review"].attrs["shape"].value == "hexagon"
        assert ("plan", "review") in {(edge.source, edge.target) for edge in merged.edges}
        assert ("review", "done") in {(edge.source, edge.target) for edge in merged.edges}

    def test_graph_merge_transform_raises_on_conflicting_existing_node_attrs(self):
        base_graph = parse_dot(
            """
            digraph Base {
                start [shape=Mdiamond]
                plan [shape=box, prompt="Primary prompt"]
                done [shape=Msquare]
                start -> plan -> done
            }
            """
        )
        module_graph = parse_dot(
            """
            digraph Module {
                plan [shape=diamond, prompt="Conflicting prompt"]
            }
            """
        )

        try:
            GraphMergeTransform(module_graph).apply(base_graph)
            assert False, "Expected merge to fail on conflicting node attrs"
        except ValueError as exc:
            assert "plan" in str(exc)
            assert "shape" in str(exc)
