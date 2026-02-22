import unittest

from attractor.dsl import parse_dot
from attractor.transforms import GoalVariableTransform, ModelStylesheetTransform, TransformPipeline


class TestTransforms(unittest.TestCase):
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
        self.assertEqual("Plan for Build API", graph.nodes["plan"].attrs["prompt"].value)

    def test_stylesheet_specificity_and_explicit_override(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="* { model = base; provider = generic; } box { model = boxy; } .fast { model = flash; } #review { model = best; provider = openai; reasoning_effort = high; }"]

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
        self.assertEqual("flash", graph.nodes["plan"].attrs["llm_model"].value)
        # explicit attribute is not overwritten
        self.assertEqual("explicit", graph.nodes["review"].attrs["llm_model"].value)
        # other properties from highest-specific rule can still apply
        self.assertEqual("openai", graph.nodes["review"].attrs["llm_provider"].value)
        self.assertEqual("high", graph.nodes["review"].attrs["reasoning_effort"].value)

    def test_transform_pipeline_order(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Landing Page", model_stylesheet="box { model = gpt-5; }"]
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

        self.assertEqual("Build Landing Page", graph.nodes["task"].attrs["prompt"].value)
        self.assertEqual("gpt-5", graph.nodes["task"].attrs["llm_model"].value)


if __name__ == "__main__":
    unittest.main()
