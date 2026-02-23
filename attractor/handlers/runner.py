from __future__ import annotations

from dataclasses import dataclass

from attractor.dsl.models import DotGraph
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome

from .base import HandlerRuntime
from .registry import HandlerRegistry


@dataclass
class HandlerRunner:
    graph: DotGraph
    registry: HandlerRegistry

    def __call__(self, node_id: str, prompt: str, context: Context) -> Outcome:
        node = self.graph.nodes[node_id]
        outgoing = [edge for edge in self.graph.edges if edge.source == node_id]
        handler = self.registry.resolve(node)
        runtime = HandlerRuntime(
            node_id=node_id,
            prompt=prompt,
            node_attrs=node.attrs,
            outgoing_edges=outgoing,
            context=context,
            graph=self.graph,
            runner=self,
        )
        return handler.run(runtime)
