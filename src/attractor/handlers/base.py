from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, List, Optional, Protocol, runtime_checkable

from attractor.dsl.models import DotAttribute, DotEdge, DotGraph, DotNode
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import Outcome
from attractor.engine.artifacts import ArtifactStore


PIPELINE_RETRY_RUN_ID_CONTEXT_KEY = "internal.pipeline_retry_run_id"


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
        provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        write_contract: ContextWriteContract | None = None,
    ) -> str | Outcome:
        ...


@dataclass
class ChildRunRequest:
    child_run_id: str
    child_graph: DotGraph
    child_flow_name: str
    child_flow_path: Path
    child_workdir: Path
    parent_context: Context
    parent_run_id: str
    parent_node_id: str
    root_run_id: str
    control: Callable[[], str | None] | None = None


@dataclass
class ChildRunResult:
    run_id: str
    status: str
    outcome: str | None = None
    outcome_reason_code: str | None = None
    outcome_reason_message: str | None = None
    current_node: str = ""
    completed_nodes: List[str] = field(default_factory=list)
    route_trace: List[str] = field(default_factory=list)
    failure_reason: str = ""


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
    child_run_launcher: Optional[Callable[[ChildRunRequest], ChildRunResult]] = None
    child_status_resolver: Optional[Callable[[str], ChildRunResult | None]] = None

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
