from attractor.dsl import parse_dot
from attractor.dsl.models import DotValueType
from attractor.transforms import GoalVariableTransform, ModelStylesheetTransform, TransformPipeline
from attractor.transforms import AttributeDefaultsTransform


class TestTransforms:
    def test_attribute_defaults_transform_injects_typed_defaults(self):
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

        assert graph.graph_attrs["goal"].value == ""
        assert graph.graph_attrs["goal"].value_type == DotValueType.STRING
        assert graph.graph_attrs["default_max_retry"].value == 50
        assert graph.graph_attrs["default_max_retry"].value_type == DotValueType.INTEGER
        assert graph.graph_attrs["default_fidelity"].value == ""
        assert graph.graph_attrs["default_fidelity"].value_type == DotValueType.STRING

        task = graph.nodes["task"]
        assert task.attrs["shape"].value == "box"
        assert task.attrs["shape"].value_type == DotValueType.STRING
        assert task.attrs["label"].value == "task"
        assert task.attrs["label"].value_type == DotValueType.STRING
        assert task.attrs["type"].value == ""
        assert task.attrs["type"].value_type == DotValueType.STRING
        assert task.attrs["prompt"].value == ""
        assert task.attrs["prompt"].value_type == DotValueType.STRING
        assert task.attrs["max_retries"].value == 0
        assert task.attrs["max_retries"].value_type == DotValueType.INTEGER
        assert task.attrs["goal_gate"].value is False
        assert task.attrs["goal_gate"].value_type == DotValueType.BOOLEAN
        assert task.attrs["retry_target"].value == ""
        assert task.attrs["retry_target"].value_type == DotValueType.STRING
        assert task.attrs["fallback_retry_target"].value == ""
        assert task.attrs["fallback_retry_target"].value_type == DotValueType.STRING
        assert task.attrs["fidelity"].value == ""
        assert task.attrs["fidelity"].value_type == DotValueType.STRING
        assert task.attrs["thread_id"].value == ""
        assert task.attrs["thread_id"].value_type == DotValueType.STRING
        assert task.attrs["class"].value == ""
        assert task.attrs["class"].value_type == DotValueType.STRING
        assert task.attrs["timeout"].value is None
        assert task.attrs["timeout"].value_type == DotValueType.DURATION
        assert task.attrs["llm_model"].value == ""
        assert task.attrs["llm_model"].value_type == DotValueType.STRING
        assert task.attrs["llm_provider"].value == ""
        assert task.attrs["llm_provider"].value_type == DotValueType.STRING
        assert task.attrs["reasoning_effort"].value == "high"
        assert task.attrs["reasoning_effort"].value_type == DotValueType.STRING
        assert task.attrs["auto_status"].value is False
        assert task.attrs["auto_status"].value_type == DotValueType.BOOLEAN
        assert task.attrs["allow_partial"].value is False
        assert task.attrs["allow_partial"].value_type == DotValueType.BOOLEAN

        assert graph.edges[0].attrs["label"].value == ""
        assert graph.edges[0].attrs["label"].value_type == DotValueType.STRING
        assert graph.edges[0].attrs["condition"].value == ""
        assert graph.edges[0].attrs["condition"].value_type == DotValueType.STRING
        assert graph.edges[0].attrs["weight"].value == 0
        assert graph.edges[0].attrs["weight"].value_type == DotValueType.INTEGER
        assert graph.edges[0].attrs["loop_restart"].value is False
        assert graph.edges[0].attrs["loop_restart"].value_type == DotValueType.BOOLEAN

    def test_attribute_defaults_transform_preserves_explicit_values(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship", default_max_retry=9]
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
        assert graph.graph_attrs["default_max_retry"].value == 9

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

    def test_stylesheet_specificity_and_explicit_override(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { llm_model: base; llm_provider: generic; } .fast { llm_model: flash; } #review { llm_model: best; llm_provider: openai; reasoning_effort: high; }"]

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
        assert graph.nodes["review"].attrs["reasoning_effort"].value == "high"

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
        pipeline.apply(graph)

        assert graph.nodes["task"].attrs["prompt"].value == "Build Landing Page"
        assert graph.nodes["task"].attrs["llm_model"].value == "gpt-5"
