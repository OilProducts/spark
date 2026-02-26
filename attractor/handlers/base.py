from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, List, Optional, Protocol, runtime_checkable

from attractor.dsl.models import DotAttribute, DotEdge, DotGraph, DotNode
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome


@runtime_checkable
class CodergenBackend(Protocol):
    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> bool:
        ...


@dataclass
class HandlerRuntime:
    node_id: str
    node: DotNode
    prompt: str
    node_attrs: Dict[str, DotAttribute]
    outgoing_edges: List[DotEdge]
    context: Context
    graph: DotGraph
    logs_root: Optional[Path]
    runner: Callable[[str, str, Context], Outcome]


class Handler(Protocol):
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        ...
