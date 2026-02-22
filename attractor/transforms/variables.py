from __future__ import annotations

from attractor.dsl.models import DotAttribute, DotGraph, DotValueType


class GoalVariableTransform:
    def apply(self, graph: DotGraph) -> DotGraph:
        goal_attr = graph.graph_attrs.get("goal")
        goal = str(goal_attr.value) if goal_attr else ""

        if goal == "":
            return graph

        for node in graph.nodes.values():
            prompt_attr = node.attrs.get("prompt")
            if not prompt_attr:
                continue

            prompt = str(prompt_attr.value)
            expanded = prompt.replace("$goal", goal)
            if expanded == prompt:
                continue

            node.attrs["prompt"] = DotAttribute(
                key="prompt",
                value=expanded,
                value_type=DotValueType.STRING,
                line=prompt_attr.line,
            )

        return graph
