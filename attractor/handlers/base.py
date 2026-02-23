from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Dict, List, Protocol

from attractor.dsl.models import DotAttribute, DotEdge, DotGraph
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome


class CodergenBackend(Protocol):
    def run(self, node_id: str, prompt: str, context: Context) -> bool:
        ...


@dataclass
class HandlerRuntime:
    node_id: str
    prompt: str
    node_attrs: Dict[str, DotAttribute]
    outgoing_edges: List[DotEdge]
    context: Context
    graph: DotGraph
    runner: Callable[[str, str, Context], Outcome]


class Handler(Protocol):
    def run(self, runtime: HandlerRuntime) -> Outcome:
        ...
