from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, List, Optional, Protocol, runtime_checkable

from attractor.dsl.models import DotAttribute, DotEdge, DotGraph, DotNode
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import Outcome
from attractor.engine.artifacts import ArtifactStore


@runtime_checkable
class CodergenBackend(Protocol):
    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: Optional[float] = None,
        model: Optional[str] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
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
    artifact_store: ArtifactStore | None
    runner: Callable[[str, str, Context], Outcome]
    event_emitter: Optional[Callable[..., None]] = None
    control: Callable[[], str | None] | None = None

    def emit(self, event_type: str, **payload: object) -> None:
        if not self.event_emitter:
            return
        try:
            self.event_emitter(event_type, **payload)
        except Exception:
            return


class Handler(Protocol):
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        ...
