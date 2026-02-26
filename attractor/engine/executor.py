from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path
import random
import threading
from typing import Callable, Dict, List, Optional, Tuple

from attractor.dsl.models import DotGraph

from .checkpoint import Checkpoint, load_checkpoint, save_checkpoint
from .context import Context
from .outcome import Outcome, OutcomeStatus
from .routing import select_next_edge


RunnerFn = Callable[[str, str, Context], Outcome | None]
ControlFn = Callable[[], Optional[str]]
EventFn = Callable[[Dict[str, object]], None]
ShouldRetryFn = Callable[[Outcome], bool]
NODE_OUTCOMES_KEY = "_attractor.node_outcomes"
RUNTIME_FIDELITY_KEY = "_attractor.runtime.fidelity"
RUNTIME_THREAD_ID_KEY = "_attractor.runtime.thread_id"
RUNTIME_CONTEXT_CARRYOVER_KEY = "_attractor.runtime.context_carryover"
_NON_CODEGEN_SHAPES = {
    "Mdiamond",
    "Msquare",
    "hexagon",
    "diamond",
    "component",
    "tripleoctagon",
    "parallelogram",
    "house",
}
GOAL_GATE_NO_RETRY_TARGET_REASON = "Goal gate unsatisfied and no retry target"


@dataclass(frozen=True)
class BackoffConfig:
    initial_delay_ms: int = 200
    backoff_factor: float = 2.0
    max_delay_ms: int = 60000
    jitter: bool = True

    def delay_for_attempt(self, attempt: int) -> float:
        normalized_attempt = max(1, attempt)
        delay = float(self.initial_delay_ms) * (self.backoff_factor ** (normalized_attempt - 1))
        delay = min(delay, float(self.max_delay_ms))
        if self.jitter:
            delay *= random.uniform(0.5, 1.5)
        return delay


def _default_should_retry_outcome(outcome: Outcome) -> bool:
    if outcome.status.value == "retry":
        return True
    if outcome.status.value == "fail":
        if outcome.retryable is not None:
            return outcome.retryable
        return True
    return False


@dataclass(frozen=True)
class RetryPolicy:
    max_attempts: int
    backoff: BackoffConfig = field(default_factory=BackoffConfig)
    should_retry: ShouldRetryFn = _default_should_retry_outcome


_RETRY_POLICY_PRESETS: Dict[str, RetryPolicy] = {
    "none": RetryPolicy(
        max_attempts=1,
        backoff=BackoffConfig(initial_delay_ms=0, backoff_factor=1.0, max_delay_ms=0, jitter=False),
    ),
    "standard": RetryPolicy(
        max_attempts=5,
        backoff=BackoffConfig(initial_delay_ms=200, backoff_factor=2.0, max_delay_ms=60000, jitter=True),
    ),
    "aggressive": RetryPolicy(
        max_attempts=5,
        backoff=BackoffConfig(initial_delay_ms=500, backoff_factor=2.0, max_delay_ms=60000, jitter=True),
    ),
    "linear": RetryPolicy(
        max_attempts=3,
        backoff=BackoffConfig(initial_delay_ms=500, backoff_factor=1.0, max_delay_ms=60000, jitter=True),
    ),
    "patient": RetryPolicy(
        max_attempts=3,
        backoff=BackoffConfig(initial_delay_ms=2000, backoff_factor=3.0, max_delay_ms=60000, jitter=True),
    ),
}


@dataclass
class PipelineResult:
    status: str
    current_node: str
    completed_nodes: List[str] = field(default_factory=list)
    context: Dict[str, object] = field(default_factory=dict)
    node_outcomes: Dict[str, Outcome] = field(default_factory=dict)
    route_trace: List[str] = field(default_factory=list)
    failure_reason: str = ""


class PipelineExecutor:
    def __init__(
        self,
        graph: DotGraph,
        runner: RunnerFn,
        logs_root: Optional[str] = None,
        checkpoint_file: Optional[str] = None,
        control: Optional[ControlFn] = None,
        on_event: Optional[EventFn] = None,
    ):
        self.graph = graph
        self.runner = runner
        self.logs_root = Path(logs_root) if logs_root else None
        self._base_logs_root = self.logs_root
        self._restart_count = 0
        self.checkpoint_path = Path(checkpoint_file) if checkpoint_file else None
        self.control = control
        self.on_event = on_event
        self._shape_start_nodes = self._node_ids_for_shape("Mdiamond")
        self._shape_exit_nodes = self._node_ids_for_shape("Msquare")
        self._active_top_level_node: str | None = None
        self._active_top_level_node_lock = threading.Lock()
        self._stage_status_transitions: Dict[str, List[str]] = {}
        self._sync_runner_logs_root()

    def run(
        self,
        context: Optional[Context] = None,
        *,
        resume: bool = False,
        max_steps: Optional[int] = None,
    ) -> PipelineResult:
        if not resume:
            self._stage_status_transitions = {}

        ctx = context or Context()
        completed: List[str] = []
        outcomes: Dict[str, Outcome] = {}
        retry_counts: Dict[str, int] = {}

        current = self._resolve_start_node()
        incoming_edge: object | None = None
        route_trace: List[str] = [current]
        degrade_resume_fidelity_once = False
        if resume and self.checkpoint_path:
            checkpoint = load_checkpoint(self.checkpoint_path)
            if checkpoint:
                candidate = checkpoint.current_node or current
                if candidate in self.graph.nodes:
                    current = candidate
                    route_trace = [current]
                    completed = [node for node in checkpoint.completed_nodes if node in self.graph.nodes]
                    retry_counts = {
                        node_id: count
                        for node_id, count in checkpoint.retry_counts.items()
                        if node_id in self.graph.nodes
                    }
                    ctx = Context(
                        values=dict(checkpoint.context),
                        logs=list(checkpoint.logs),
                    )
                    degrade_resume_fidelity_once = (
                        str(checkpoint.context.get(RUNTIME_FIDELITY_KEY, "")).strip().lower() == "full"
                    )
                    if current in completed:
                        resumed_next = self._resolve_resume_next_edge(current, ctx)
                        if resumed_next is not None:
                            current = resumed_next.target
                            incoming_edge = resumed_next
                            route_trace = [current]
        self._mirror_graph_goal(ctx)
        self._seed_builtin_context(ctx, current)
        self._emit_event("PipelineStarted", current_node=current, resumed=resume)

        steps = 0
        try:
            while True:
                ctx.set("current_node", current)
                action = self._poll_control()
                if action == "abort":
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineFailed",
                        error="aborted_by_user",
                    )
                    return PipelineResult(
                        status="aborted",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                        failure_reason="aborted_by_user",
                    )
                if action == "pause":
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelinePaused",
                    )
                    return PipelineResult(
                        status="paused",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                    )

                if self._is_exit_node(current):
                    gates_ok, failed_gate_node = self._check_goal_gates(ctx, completed)
                    if gates_ok:
                        self._finalize_run(
                            current_node=current,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                            event_type="PipelineCompleted",
                        )
                        return PipelineResult(
                            status="success",
                            current_node=current,
                            completed_nodes=completed,
                            context=dict(ctx.values),
                            node_outcomes=outcomes,
                            route_trace=route_trace,
                        )

                    retry_target = self._resolve_goal_gate_retry_target(failed_gate_node)
                    if retry_target:
                        current = retry_target
                        ctx.set("current_node", current)
                        incoming_edge = None
                        route_trace.append(current)
                        self._save_checkpoint(
                            current_node=current,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                        )
                        continue

                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineFailed",
                        error=GOAL_GATE_NO_RETRY_TARGET_REASON,
                    )
                    return PipelineResult(
                        status="fail",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                        failure_reason=GOAL_GATE_NO_RETRY_TARGET_REASON,
                    )

                node = self.graph.nodes[current]
                fidelity = self._resolve_runtime_fidelity(node.node_id, incoming_edge)
                if degrade_resume_fidelity_once:
                    fidelity = "summary:high"
                    degrade_resume_fidelity_once = False
                ctx.set(RUNTIME_FIDELITY_KEY, fidelity)
                ctx.set(RUNTIME_THREAD_ID_KEY, self._resolve_runtime_thread_id(node.node_id, incoming_edge, fidelity))
                ctx.set(
                    RUNTIME_CONTEXT_CARRYOVER_KEY,
                    self._build_runtime_context_carryover(fidelity, ctx, completed),
                )
                prior_status = self._context_outcome_status(ctx)
                prior_preferred_label = self._context_preferred_label(ctx)
                prompt = self._prompt_for_node(node.node_id)
                self._emit_event("StageStarted", node_id=node.node_id, index=len(completed))
                self._enter_top_level_stage(node.node_id)
                try:
                    outcome = self._execute_node_handler(node.node_id, prompt, ctx)
                finally:
                    self._exit_top_level_stage()
                outcomes[node.node_id] = outcome

                if outcome.context_updates:
                    ctx.merge_updates(outcome.context_updates)
                ctx.set("outcome", outcome.status.value)
                ctx.set("preferred_label", outcome.preferred_label or "")
                self._remember_node_outcome(ctx, node.node_id, outcome.status.value)

                self._write_stage_artifacts(node.node_id, prompt, outcome)

                retry_policy = self._retry_policy_for_node(node.node_id)
                max_retries = retry_policy.max_attempts - 1
                retries_so_far = retry_counts.get(node.node_id, 0)
                if self._should_retry(outcome, retries_so_far, retry_policy):
                    retry_counts[node.node_id] = retries_so_far + 1
                    delay_ms = retry_policy.backoff.delay_for_attempt(retry_counts[node.node_id])
                    self._emit_event(
                        "StageRetrying",
                        node_id=node.node_id,
                        index=len(completed),
                        attempt=retry_counts[node.node_id],
                        delay=delay_ms,
                    )
                    self._save_checkpoint(
                        current_node=node.node_id,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                    )
                    continue

                original_status = outcome.status
                outcome = self._coerce_retry_exhausted_outcome(
                    node.node_id,
                    outcome,
                    retries_so_far,
                    max_retries,
                )
                if outcome.status != original_status:
                    outcomes[node.node_id] = outcome
                    ctx.set("outcome", outcome.status.value)
                    ctx.set("preferred_label", outcome.preferred_label or "")
                    self._remember_node_outcome(ctx, node.node_id, outcome.status.value)
                    self._write_stage_artifacts(node.node_id, prompt, outcome)
                self._reset_retry_counter(node.node_id, outcome, retry_counts)

                if outcome.status.value == "fail":
                    self._emit_event(
                        "StageFailed",
                        node_id=node.node_id,
                        index=len(completed),
                        error=outcome.failure_reason or "stage_failed",
                        will_retry=False,
                    )
                else:
                    self._emit_event(
                        "StageCompleted",
                        node_id=node.node_id,
                        index=len(completed),
                        outcome=outcome.status.value,
                    )
                completed.append(node.node_id)
                self._save_checkpoint(
                    current_node=node.node_id,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
                )
                outgoing = [edge for edge in self.graph.edges if edge.source == node.node_id]
                routing_outcome = self._routing_outcome(
                    node.node_id,
                    outcome,
                    prior_status,
                    prior_preferred_label,
                )
                next_edge = self._select_route_edge(node.node_id, outgoing, routing_outcome, ctx)
                if not next_edge:
                    if routing_outcome.status == OutcomeStatus.FAIL:
                        failure_reason = self._stage_failure_reason(routing_outcome)
                        self._emit_event(
                            "StageFailed",
                            node_id=node.node_id,
                            index=len(completed),
                            error=failure_reason,
                            will_retry=False,
                        )
                        self._finalize_run(
                            current_node=node.node_id,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                            event_type="PipelineFailed",
                            error=failure_reason,
                        )
                        return PipelineResult(
                            status="fail",
                            current_node=node.node_id,
                            completed_nodes=completed,
                            context=dict(ctx.values),
                            node_outcomes=outcomes,
                            route_trace=route_trace,
                            failure_reason=failure_reason,
                        )
                    message = self._no_route_message(node.node_id, routing_outcome)
                    self._emit_event(
                        "StageFailed",
                        node_id=node.node_id,
                        index=len(completed),
                        error=message,
                        will_retry=False,
                    )
                    raise RuntimeError(message)

                if _edge_attr_bool(next_edge, "loop_restart"):
                    self._emit_event("PipelineRestarted", from_node=node.node_id, restart_node=next_edge.target)
                    self._apply_loop_restart(
                        restart_node=next_edge.target,
                        context=ctx,
                        completed=completed,
                        outcomes=outcomes,
                        retry_counts=retry_counts,
                        route_trace=route_trace,
                        persist_checkpoint=True,
                    )
                    current = next_edge.target
                    incoming_edge = None
                    steps = 0
                    continue

                current = next_edge.target
                ctx.set("current_node", current)
                incoming_edge = next_edge
                route_trace.append(current)
                self._save_checkpoint(
                    current_node=current,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
                )

                steps += 1
                if max_steps is not None and steps >= max_steps:
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelinePaused",
                    )
                    return PipelineResult(
                        status="paused",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                    )
        except Exception as exc:
            self._finalize_run(
                current_node=current,
                completed_nodes=completed,
                context=ctx,
                retry_counts=retry_counts,
                event_type="PipelineFailed",
                error=str(exc),
            )
            raise

    def run_from(
        self,
        start_node: str,
        context: Optional[Context] = None,
        *,
        max_steps: Optional[int] = None,
        stop_nodes: Optional[set[str]] = None,
    ) -> PipelineResult:
        self._stage_status_transitions = {}

        ctx = context or Context()
        self._mirror_graph_goal(ctx)
        completed: List[str] = []
        outcomes: Dict[str, Outcome] = {}
        retry_counts: Dict[str, int] = {}
        current = start_node
        self._seed_builtin_context(ctx, current)
        incoming_edge: object | None = None
        route_trace: List[str] = [current]
        steps = 0
        stop_nodes = set(stop_nodes or [])
        self._emit_event("PipelineStarted", current_node=current, resumed=False)

        try:
            while True:
                ctx.set("current_node", current)
                action = self._poll_control()
                if action == "abort":
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineFailed",
                        error="aborted_by_user",
                    )
                    return PipelineResult(
                        status="aborted",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                        failure_reason="aborted_by_user",
                    )
                if action == "pause":
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelinePaused",
                    )
                    return PipelineResult(
                        status="paused",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                    )

                if self._is_exit_node(current):
                    gates_ok, failed_gate_node = self._check_goal_gates(ctx, completed)
                    if gates_ok:
                        self._finalize_run(
                            current_node=current,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                            event_type="PipelineCompleted",
                        )
                        return PipelineResult(
                            status="success",
                            current_node=current,
                            completed_nodes=completed,
                            context=dict(ctx.values),
                            node_outcomes=outcomes,
                            route_trace=route_trace,
                        )

                    retry_target = self._resolve_goal_gate_retry_target(failed_gate_node)
                    if retry_target:
                        current = retry_target
                        ctx.set("current_node", current)
                        incoming_edge = None
                        route_trace.append(current)
                        self._save_checkpoint(
                            current_node=current,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                        )
                        continue

                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineFailed",
                        error=GOAL_GATE_NO_RETRY_TARGET_REASON,
                    )
                    return PipelineResult(
                        status="fail",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                        failure_reason=GOAL_GATE_NO_RETRY_TARGET_REASON,
                    )

                if current in stop_nodes:
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineCompleted",
                    )
                    return PipelineResult(
                        status="success",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                    )

                node = self.graph.nodes[current]
                fidelity = self._resolve_runtime_fidelity(node.node_id, incoming_edge)
                ctx.set(RUNTIME_FIDELITY_KEY, fidelity)
                ctx.set(RUNTIME_THREAD_ID_KEY, self._resolve_runtime_thread_id(node.node_id, incoming_edge, fidelity))
                ctx.set(
                    RUNTIME_CONTEXT_CARRYOVER_KEY,
                    self._build_runtime_context_carryover(fidelity, ctx, completed),
                )
                prior_status = self._context_outcome_status(ctx)
                prior_preferred_label = self._context_preferred_label(ctx)
                prompt = self._prompt_for_node(node.node_id)
                self._emit_event("StageStarted", node_id=node.node_id, index=len(completed))
                self._enter_top_level_stage(node.node_id)
                try:
                    outcome = self._execute_node_handler(node.node_id, prompt, ctx)
                finally:
                    self._exit_top_level_stage()
                outcomes[node.node_id] = outcome

                if outcome.context_updates:
                    ctx.merge_updates(outcome.context_updates)
                ctx.set("outcome", outcome.status.value)
                ctx.set("preferred_label", outcome.preferred_label or "")
                self._remember_node_outcome(ctx, node.node_id, outcome.status.value)

                self._write_stage_artifacts(node.node_id, prompt, outcome)

                retry_policy = self._retry_policy_for_node(node.node_id)
                max_retries = retry_policy.max_attempts - 1
                retries_so_far = retry_counts.get(node.node_id, 0)
                if self._should_retry(outcome, retries_so_far, retry_policy):
                    retry_counts[node.node_id] = retries_so_far + 1
                    delay_ms = retry_policy.backoff.delay_for_attempt(retry_counts[node.node_id])
                    self._emit_event(
                        "StageRetrying",
                        node_id=node.node_id,
                        index=len(completed),
                        attempt=retry_counts[node.node_id],
                        delay=delay_ms,
                    )
                    continue

                original_status = outcome.status
                outcome = self._coerce_retry_exhausted_outcome(
                    node.node_id,
                    outcome,
                    retries_so_far,
                    max_retries,
                )
                if outcome.status != original_status:
                    outcomes[node.node_id] = outcome
                    ctx.set("outcome", outcome.status.value)
                    ctx.set("preferred_label", outcome.preferred_label or "")
                    self._remember_node_outcome(ctx, node.node_id, outcome.status.value)
                    self._write_stage_artifacts(node.node_id, prompt, outcome)
                self._reset_retry_counter(node.node_id, outcome, retry_counts)

                if outcome.status.value == "fail":
                    self._emit_event(
                        "StageFailed",
                        node_id=node.node_id,
                        index=len(completed),
                        error=outcome.failure_reason or "stage_failed",
                        will_retry=False,
                    )
                else:
                    self._emit_event(
                        "StageCompleted",
                        node_id=node.node_id,
                        index=len(completed),
                        outcome=outcome.status.value,
                    )
                completed.append(node.node_id)
                self._save_checkpoint(
                    current_node=node.node_id,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
                )
                outgoing = [edge for edge in self.graph.edges if edge.source == node.node_id]
                routing_outcome = self._routing_outcome(
                    node.node_id,
                    outcome,
                    prior_status,
                    prior_preferred_label,
                )
                next_edge = self._select_route_edge(node.node_id, outgoing, routing_outcome, ctx)
                if not next_edge:
                    if routing_outcome.status == OutcomeStatus.FAIL:
                        failure_reason = self._stage_failure_reason(routing_outcome)
                        self._emit_event(
                            "StageFailed",
                            node_id=node.node_id,
                            index=len(completed),
                            error=failure_reason,
                            will_retry=False,
                        )
                        self._finalize_run(
                            current_node=node.node_id,
                            completed_nodes=completed,
                            context=ctx,
                            retry_counts=retry_counts,
                            event_type="PipelineFailed",
                            error=failure_reason,
                        )
                        return PipelineResult(
                            status="fail",
                            current_node=node.node_id,
                            completed_nodes=completed,
                            context=dict(ctx.values),
                            node_outcomes=outcomes,
                            route_trace=route_trace,
                            failure_reason=failure_reason,
                        )
                    message = self._no_route_message(node.node_id, routing_outcome)
                    self._finalize_run(
                        current_node=node.node_id,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelineFailed",
                        error=message,
                    )
                    return PipelineResult(
                        status="fail",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                        failure_reason=message,
                    )

                if _edge_attr_bool(next_edge, "loop_restart"):
                    self._emit_event("PipelineRestarted", from_node=node.node_id, restart_node=next_edge.target)
                    self._apply_loop_restart(
                        restart_node=next_edge.target,
                        context=ctx,
                        completed=completed,
                        outcomes=outcomes,
                        retry_counts=retry_counts,
                        route_trace=route_trace,
                        persist_checkpoint=False,
                    )
                    current = next_edge.target
                    incoming_edge = None
                    steps = 0
                    continue

                current = next_edge.target
                ctx.set("current_node", current)
                incoming_edge = next_edge
                route_trace.append(current)

                steps += 1
                if max_steps is not None and steps >= max_steps:
                    self._finalize_run(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                        event_type="PipelinePaused",
                    )
                    return PipelineResult(
                        status="paused",
                        current_node=current,
                        completed_nodes=completed,
                        context=dict(ctx.values),
                        node_outcomes=outcomes,
                        route_trace=route_trace,
                    )
        except Exception as exc:
            self._finalize_run(
                current_node=current,
                completed_nodes=completed,
                context=ctx,
                retry_counts=retry_counts,
                event_type="PipelineFailed",
                error=str(exc),
            )
            raise

    def _save_checkpoint(
        self,
        current_node: str,
        completed_nodes: List[str],
        context: Context,
        retry_counts: Dict[str, int],
    ) -> None:
        persisted = bool(self.checkpoint_path)
        if self.checkpoint_path:
            checkpoint = Checkpoint(
                current_node=current_node,
                completed_nodes=list(completed_nodes),
                context=dict(context.values),
                retry_counts=dict(retry_counts),
                logs=list(context.clone().logs),
            )
            save_checkpoint(self.checkpoint_path, checkpoint)
        self._emit_event("CheckpointSaved", node_id=current_node, persisted=persisted)

    def _finalize_run(
        self,
        *,
        current_node: str,
        completed_nodes: List[str],
        context: Context,
        retry_counts: Dict[str, int],
        event_type: str,
        error: str = "",
    ) -> None:
        self._save_checkpoint(
            current_node=current_node,
            completed_nodes=completed_nodes,
            context=context,
            retry_counts=retry_counts,
        )
        if error:
            self._emit_event(event_type, current_node=current_node, error=error)
        else:
            self._emit_event(event_type, current_node=current_node)
        self._cleanup_resources()

    def _cleanup_resources(self) -> None:
        self._close_if_supported(self.runner)
        self._close_if_supported(self.control)

    def _close_if_supported(self, target: object) -> None:
        if target is None:
            return
        close_fn = getattr(target, "close", None)
        if not callable(close_fn):
            return
        try:
            close_fn()
        except Exception:
            return

    def _poll_control(self) -> Optional[str]:
        if not self.control:
            return None
        try:
            return self.control()
        except Exception:
            return None

    def _emit_event(self, event_type: str, **payload: object) -> None:
        if not self.on_event:
            return
        event: Dict[str, object] = {"type": event_type}
        event.update(payload)
        try:
            self.on_event(event)
        except Exception:
            return

    def _write_stage_artifacts(self, node_id: str, prompt: str, outcome: Outcome) -> None:
        if not self.logs_root:
            return

        transitions = self._stage_status_transitions.setdefault(node_id, [])
        transitions.append(outcome.status.value)

        stage_dir = self.logs_root / node_id
        stage_dir.mkdir(parents=True, exist_ok=True)

        (stage_dir / "prompt.md").write_text(prompt + "\n", encoding="utf-8")
        response_text = outcome.notes or ""
        (stage_dir / "response.md").write_text(response_text + "\n", encoding="utf-8")

        status_payload = {
            "outcome": outcome.status.value,
            "preferred_next_label": outcome.preferred_label,
            "suggested_next_ids": list(outcome.suggested_next_ids),
            "context_updates": dict(outcome.context_updates),
            "notes": outcome.notes,
            "status_transitions": list(transitions),
        }
        with (stage_dir / "status.json").open("w", encoding="utf-8") as f:
            json.dump(status_payload, f, indent=2, sort_keys=True)

    def _apply_loop_restart(
        self,
        restart_node: str,
        context: Context,
        completed: List[str],
        outcomes: Dict[str, Outcome],
        retry_counts: Dict[str, int],
        route_trace: List[str],
        *,
        persist_checkpoint: bool,
    ) -> None:
        completed.clear()
        outcomes.clear()
        retry_counts.clear()
        route_trace.clear()
        route_trace.append(restart_node)

        context.set(NODE_OUTCOMES_KEY, {})
        context.set("outcome", "")
        context.set("preferred_label", "")
        context.set("current_node", restart_node)

        self._rotate_logs_root_for_restart()

        if persist_checkpoint:
            self._save_checkpoint(
                current_node=restart_node,
                completed_nodes=completed,
                context=context,
                retry_counts=retry_counts,
            )

        self._emit_event("PipelineStarted", current_node=restart_node, resumed=False, restarted=True)

    def _rotate_logs_root_for_restart(self) -> None:
        if not self._base_logs_root:
            return

        while True:
            self._restart_count += 1
            candidate = self._base_logs_root.parent / f"{self._base_logs_root.name}.restart-{self._restart_count}"
            if candidate.exists():
                continue
            candidate.mkdir(parents=True, exist_ok=True)
            self.logs_root = candidate
            self._sync_runner_logs_root()
            return

    def _sync_runner_logs_root(self) -> None:
        setter = getattr(self.runner, "set_logs_root", None)
        if callable(setter):
            setter(self.logs_root)

    def _resolve_start_node(self) -> str:
        starts = [node.node_id for node in self.graph.nodes.values() if self._is_start_node(node.node_id)]
        if not starts:
            raise RuntimeError("No start node found; expected shape=Mdiamond or node id start/Start")
        if len(starts) > 1:
            raise RuntimeError(f"Ambiguous start nodes: {', '.join(sorted(starts))}")
        return starts[0]

    def _normalize_outcome(self, node_id: str, outcome: Outcome | None) -> Outcome:
        if isinstance(outcome, Outcome):
            return outcome

        node = self.graph.nodes[node_id]
        auto_status_attr = node.attrs.get("auto_status")
        if auto_status_attr and _to_bool(auto_status_attr.value):
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="auto-status: handler completed without writing status",
            )

        return Outcome(status=OutcomeStatus.FAIL, failure_reason="handler returned no outcome")

    def _execute_node_handler(self, node_id: str, prompt: str, context: Context) -> Outcome:
        try:
            raw_outcome = self.runner(node_id, prompt, context)
        except BaseException as exc:
            if isinstance(exc, KeyboardInterrupt):
                raise
            failure_reason = str(exc) or exc.__class__.__name__
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_reason=failure_reason,
                retryable=_is_retryable_exception(exc) if isinstance(exc, Exception) else False,
            )
        return self._normalize_outcome(node_id, raw_outcome)

    def _enter_top_level_stage(self, node_id: str) -> None:
        with self._active_top_level_node_lock:
            if self._active_top_level_node is not None:
                raise RuntimeError(
                    "Top-level traversal must be single-threaded; "
                    f"attempted to run '{node_id}' while '{self._active_top_level_node}' was still executing"
                )
            self._active_top_level_node = node_id

    def _exit_top_level_stage(self) -> None:
        with self._active_top_level_node_lock:
            self._active_top_level_node = None

    def _is_start_node(self, node_id: str) -> bool:
        if self._shape_start_nodes:
            return node_id in self._shape_start_nodes
        return node_id in {"start", "Start"}

    def _is_exit_node(self, node_id: str) -> bool:
        if self._shape_exit_nodes:
            return node_id in self._shape_exit_nodes
        return node_id in {"exit", "end", "Exit", "End"}

    def _is_conditional_node(self, node_id: str) -> bool:
        node = self.graph.nodes[node_id]
        explicit = node.attrs.get("type")
        if explicit and str(explicit.value).strip():
            return str(explicit.value).strip() == "conditional"
        shape = node.attrs.get("shape")
        return bool(shape and str(shape.value) == "diamond")

    def _context_outcome_status(self, context: Context) -> Optional[OutcomeStatus]:
        value = context.get("outcome", "")
        if isinstance(value, OutcomeStatus):
            return value
        text = str(value).strip().lower()
        try:
            return OutcomeStatus(text)
        except ValueError:
            return None

    def _routing_outcome(
        self,
        node_id: str,
        outcome: Outcome,
        prior_status: Optional[OutcomeStatus],
        prior_preferred_label: str,
    ) -> Outcome:
        if not prior_status or not self._is_conditional_node(node_id):
            return outcome
        return Outcome(
            status=prior_status,
            preferred_label=prior_preferred_label,
            suggested_next_ids=list(outcome.suggested_next_ids),
            context_updates=dict(outcome.context_updates),
            failure_reason=outcome.failure_reason,
            notes=outcome.notes,
        )

    def _context_preferred_label(self, context: Context) -> str:
        return str(context.get("preferred_label", "")).strip()

    def _node_ids_for_shape(self, shape: str) -> set[str]:
        matches: set[str] = set()
        for node in self.graph.nodes.values():
            shape_attr = node.attrs.get("shape")
            if shape_attr and str(shape_attr.value) == shape:
                matches.add(node.node_id)
        return matches

    def _prompt_for_node(self, node_id: str) -> str:
        node = self.graph.nodes[node_id]
        prompt_attr = node.attrs.get("prompt")
        prompt_text = str(prompt_attr.value) if prompt_attr else ""
        if prompt_text.strip():
            return prompt_text

        if self._resolved_handler_type(node_id) == "codergen":
            label_attr = node.attrs.get("label")
            if label_attr:
                label = str(label_attr.value).strip()
                if label:
                    return label
            return node_id

        return prompt_text

    def _resolved_handler_type(self, node_id: str) -> str:
        node = self.graph.nodes[node_id]
        explicit = node.attrs.get("type")
        if explicit and str(explicit.value).strip():
            return str(explicit.value).strip()

        shape = node.attrs.get("shape")
        if shape and str(shape.value).strip() in _NON_CODEGEN_SHAPES:
            return "non-codergen"
        return "codergen"

    def _mirror_graph_goal(self, context: Context) -> None:
        goal_attr = self.graph.graph_attrs.get("goal")
        context.set("graph.goal", str(goal_attr.value) if goal_attr else "")

    def _seed_builtin_context(self, context: Context, current_node: str) -> None:
        outcomes = context.get(NODE_OUTCOMES_KEY, None)
        if not isinstance(outcomes, dict):
            context.set(NODE_OUTCOMES_KEY, {})
        if context.get("outcome", None) is None:
            context.set("outcome", "")
        if context.get("preferred_label", None) is None:
            context.set("preferred_label", "")
        context.set("current_node", current_node)

    def _resolve_runtime_fidelity(self, node_id: str, incoming_edge: object | None) -> str:
        edge_fidelity = _edge_attr_text(incoming_edge, "fidelity")
        if edge_fidelity:
            return edge_fidelity

        node_attr = self.graph.nodes[node_id].attrs.get("fidelity")
        if node_attr and str(node_attr.value).strip():
            return str(node_attr.value).strip()

        graph_attr = self.graph.graph_attrs.get("default_fidelity")
        if graph_attr and str(graph_attr.value).strip():
            return str(graph_attr.value).strip()

        return "compact"

    def _resolve_runtime_thread_id(
        self,
        node_id: str,
        incoming_edge: object | None,
        fidelity: str,
    ) -> str:
        if fidelity != "full":
            return ""

        node_attr = self.graph.nodes[node_id].attrs.get("thread_id")
        if node_attr and str(node_attr.value).strip():
            return str(node_attr.value).strip()

        edge_thread_id = _edge_attr_text(incoming_edge, "thread_id")
        if edge_thread_id:
            return edge_thread_id

        graph_attr = self.graph.graph_attrs.get("thread_id")
        if graph_attr and str(graph_attr.value).strip():
            return str(graph_attr.value).strip()

        class_attr = self.graph.nodes[node_id].attrs.get("class")
        if class_attr:
            classes = [part.strip() for part in str(class_attr.value).split(",") if part.strip()]
            if classes:
                return classes[0]

        previous_node_id = _edge_endpoint_text(incoming_edge, "source")
        if previous_node_id:
            return previous_node_id
        return node_id

    def _build_runtime_context_carryover(
        self,
        fidelity: str,
        context: Context,
        completed_nodes: List[str],
    ) -> str:
        mode = fidelity.strip().lower()
        if mode == "full":
            return ""

        goal = str(context.get("graph.goal", "")).strip()
        run_id = str(context.get("internal.run_id", "")).strip()
        statuses = context.get(NODE_OUTCOMES_KEY, {})
        if not isinstance(statuses, dict):
            statuses = {}

        if mode == "truncate":
            return "\n".join(
                [
                    "carryover:truncate",
                    f"goal={goal}",
                    f"run_id={run_id}",
                ]
            )

        context_items = self._carryover_context_items(context.snapshot())
        if mode == "compact":
            lines = [
                "carryover:compact",
                f"goal={goal}",
                f"run_id={run_id}",
            ]
            if completed_nodes:
                completed_summary = ", ".join(
                    f"{node_id}:{statuses.get(node_id, '')}" for node_id in completed_nodes
                )
                lines.append(f"completed={completed_summary}")
            for key, value in context_items[:8]:
                lines.append(f"- {key}={value}")
            return "\n".join(lines)

        summary_limits = {
            "summary:low": (3, 4),
            "summary:medium": (6, 8),
            "summary:high": (12, 16),
        }
        if mode in summary_limits:
            max_stages, max_context_items = summary_limits[mode]
            recent_nodes = completed_nodes[-max_stages:]
            recent_summary = ", ".join(
                f"{node_id}:{statuses.get(node_id, '')}" for node_id in recent_nodes
            )
            lines = [
                f"carryover:{mode}",
                f"goal={goal}",
                f"run_id={run_id}",
                f"recent_stages={recent_summary}",
                f"log_entries={len(context.logs)}",
            ]
            for key, value in context_items[:max_context_items]:
                lines.append(f"{key}={value}")
            return "\n".join(lines)

        return self._build_runtime_context_carryover("compact", context, completed_nodes)

    def _carryover_context_items(self, snapshot: Dict[str, object]) -> List[Tuple[str, str]]:
        items: List[Tuple[str, str]] = []
        for key in sorted(snapshot.keys()):
            if key in {"graph.goal", "internal.run_id"}:
                continue
            if key.startswith("_attractor.") or key.startswith("internal."):
                continue
            items.append((key, _to_context_text(snapshot[key])))
        return items

    def _max_retries_for_node(self, node_id: str) -> int:
        node = self.graph.nodes[node_id]
        node_attr = node.attrs.get("max_retries")
        if node_attr and node_attr.line > 0:
            return _to_int(node_attr.value, 0)

        graph_attr = self.graph.graph_attrs.get("default_max_retry")
        if graph_attr:
            return _to_int(graph_attr.value, 50)

        if node_attr:
            return _to_int(node_attr.value, 0)
        return 50

    def _retry_policy_for_node(self, node_id: str) -> RetryPolicy:
        preset_name = self._retry_policy_name_for_node(node_id)
        if preset_name:
            preset = _RETRY_POLICY_PRESETS.get(preset_name)
            if preset is not None:
                return preset

        max_retries = self._max_retries_for_node(node_id)
        max_attempts = max(1, max_retries + 1)
        return RetryPolicy(max_attempts=max_attempts)

    def _retry_policy_name_for_node(self, node_id: str) -> str:
        node = self.graph.nodes[node_id]
        attr = node.attrs.get("retry_policy")
        if not attr:
            return ""
        return str(attr.value).strip().lower()

    def _coerce_retry_exhausted_outcome(
        self,
        node_id: str,
        outcome: Outcome,
        retries_so_far: int,
        max_retries: int,
    ) -> Outcome:
        if outcome.status not in {OutcomeStatus.RETRY, OutcomeStatus.FAIL}:
            return outcome
        if retries_so_far < max_retries:
            return outcome

        allow_partial_attr = self.graph.nodes[node_id].attrs.get("allow_partial")
        if not allow_partial_attr or not _to_bool(allow_partial_attr.value):
            if outcome.status == OutcomeStatus.FAIL:
                return outcome
            return Outcome(
                status=OutcomeStatus.FAIL,
                preferred_label=outcome.preferred_label,
                suggested_next_ids=list(outcome.suggested_next_ids),
                context_updates=dict(outcome.context_updates),
                failure_reason="max retries exceeded",
                notes=outcome.notes,
            )

        if outcome.status == OutcomeStatus.FAIL and outcome.retryable is False:
            return outcome

        return Outcome(
            status=OutcomeStatus.PARTIAL_SUCCESS,
            preferred_label=outcome.preferred_label,
            suggested_next_ids=list(outcome.suggested_next_ids),
            context_updates=dict(outcome.context_updates),
            notes=outcome.notes or "retries exhausted, partial accepted",
        )

    def _should_retry(self, outcome: Outcome, retries_so_far: int, policy: RetryPolicy) -> bool:
        if retries_so_far + 1 >= policy.max_attempts:
            return False
        return policy.should_retry(outcome)

    def _reset_retry_counter(self, node_id: str, outcome: Outcome, retry_counts: Dict[str, int]) -> None:
        if outcome.status not in {OutcomeStatus.SUCCESS, OutcomeStatus.PARTIAL_SUCCESS}:
            return
        retry_counts.pop(node_id, None)

    def _resolve_failure_retry_target(self, node_id: str) -> str:
        node = self.graph.nodes[node_id]
        for key in ("retry_target", "fallback_retry_target"):
            attr = node.attrs.get(key)
            if attr:
                target = str(attr.value).strip()
                if target in self.graph.nodes:
                    return target
        return ""

    def _resolve_graph_retry_target(self) -> str:
        for key in ("retry_target", "fallback_retry_target"):
            attr = self.graph.graph_attrs.get(key)
            if attr:
                target = str(attr.value)
                if target in self.graph.nodes:
                    return target
        return ""

    def _select_route_edge(
        self,
        node_id: str,
        outgoing: List[DotEdge],
        routing_outcome: Outcome,
        context: Context,
    ) -> DotEdge | _SyntheticEdge | None:
        next_edge = select_next_edge(outgoing, routing_outcome, context)
        if routing_outcome.status.value != "fail":
            return next_edge

        fail_edges = [
            edge for edge in outgoing if _is_outcome_fail_condition(_edge_attr_text(edge, "condition"))
        ]
        if fail_edges:
            prioritized = select_next_edge(fail_edges, routing_outcome, context)
            if prioritized:
                return prioritized

        route = self._resolve_failure_retry_target(node_id)
        if route:
            return _SyntheticEdge(route)

        return next_edge

    def _resolve_resume_next_edge(self, node_id: str, context: Context) -> DotEdge | _SyntheticEdge | None:
        outgoing = [edge for edge in self.graph.edges if edge.source == node_id]
        if not outgoing:
            return None
        status = self._resume_status_for_node(node_id, context)
        preferred_label = self._context_preferred_label(context)
        return self._select_route_edge(
            node_id,
            outgoing,
            Outcome(status=status, preferred_label=preferred_label),
            context,
        )

    def _resume_status_for_node(self, node_id: str, context: Context) -> OutcomeStatus:
        stored = context.get(NODE_OUTCOMES_KEY, {})
        if isinstance(stored, dict):
            parsed = self._parse_outcome_status(stored.get(node_id, ""))
            if parsed is not None:
                return parsed
        parsed = self._context_outcome_status(context)
        if parsed is not None:
            return parsed
        return OutcomeStatus.SUCCESS

    def _parse_outcome_status(self, value: object) -> OutcomeStatus | None:
        if isinstance(value, OutcomeStatus):
            return value
        text = str(value).strip().lower()
        try:
            return OutcomeStatus(text)
        except ValueError:
            return None

    def _remember_node_outcome(self, context: Context, node_id: str, status: str) -> None:
        stored = context.get(NODE_OUTCOMES_KEY, {})
        if not isinstance(stored, dict):
            stored = {}
        stored = dict(stored)
        stored[node_id] = status
        context.set(NODE_OUTCOMES_KEY, stored)

    def _check_goal_gates(self, context: Context, completed_nodes: List[str]) -> Tuple[bool, str]:
        statuses = context.get(NODE_OUTCOMES_KEY, {})
        if not isinstance(statuses, dict):
            statuses = {}

        visited = set(completed_nodes)
        for node_id, status in statuses.items():
            if node_id not in visited:
                continue
            node = self.graph.nodes.get(node_id)
            if node is None:
                continue
            goal_gate_attr = node.attrs.get("goal_gate")
            if not goal_gate_attr:
                continue
            if not _to_bool(goal_gate_attr.value):
                continue

            if status not in {"success", "partial_success"}:
                return False, node_id

        return True, ""

    def _resolve_goal_gate_retry_target(self, failed_gate_node: str) -> str:
        if not failed_gate_node:
            return ""
        node_target = self._resolve_failure_retry_target(failed_gate_node)
        if node_target:
            return node_target
        return self._resolve_graph_retry_target()

    def _no_route_message(self, node_id: str, routing_outcome: Outcome) -> str:
        if routing_outcome.status == OutcomeStatus.FAIL:
            return f"Stage '{node_id}' failed with no outgoing fail edge"
        return f"Stage '{node_id}' has no eligible outgoing edge"

    def _stage_failure_reason(self, outcome: Outcome) -> str:
        reason = (outcome.failure_reason or "").strip()
        if reason:
            return reason
        return "stage_failed"


class _SyntheticEdge:
    def __init__(self, target: str):
        self.target = target


def _to_int(value: object, default: int) -> int:
    if isinstance(value, int):
        return value
    try:
        return int(str(value))
    except (TypeError, ValueError):
        return default


def _to_bool(value: object) -> bool:
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() == "true"


def _to_context_text(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float, str)):
        return str(value)
    return json.dumps(value, sort_keys=True)


def _edge_attr_text(edge: object | None, key: str) -> str:
    if edge is None:
        return ""
    attrs = getattr(edge, "attrs", None)
    if not isinstance(attrs, dict):
        return ""
    attr = attrs.get(key)
    if not attr:
        return ""
    return str(attr.value).strip()


def _edge_attr_bool(edge: object | None, key: str) -> bool:
    if edge is None:
        return False
    attrs = getattr(edge, "attrs", None)
    if not isinstance(attrs, dict):
        return False
    attr = attrs.get(key)
    if not attr:
        return False
    return _to_bool(attr.value)


def _edge_endpoint_text(edge: object | None, key: str) -> str:
    if edge is None or not hasattr(edge, key):
        return ""
    value = getattr(edge, key)
    if value is None:
        return ""
    return str(value).strip()


def _is_outcome_fail_condition(condition: str) -> bool:
    return "".join(condition.split()).lower() == "outcome=fail"


def _is_retryable_exception(exc: Exception) -> bool:
    status_code = _extract_status_code(exc)
    if status_code in {400, 401, 403}:
        return False
    if status_code == 429:
        return True
    if status_code is not None and 500 <= status_code <= 599:
        return True

    class_name = exc.__class__.__name__.strip().lower()
    non_retryable_class_tokens = (
        "authentication",
        "authorization",
        "permission",
        "forbidden",
        "badrequest",
        "validation",
        "invalid",
        "configuration",
        "config",
    )
    if any(token in class_name for token in non_retryable_class_tokens):
        return False

    retryable_class_tokens = (
        "ratelimit",
        "timeout",
        "network",
        "connection",
        "transient",
        "temporary",
        "unavailable",
        "servererror",
        "toomanyrequests",
    )
    if any(token in class_name for token in retryable_class_tokens):
        return True

    text = f"{exc.__class__.__name__} {exc}".strip().lower()

    non_retryable_tokens = (
        "401",
        "403",
        "400",
        "unauthorized",
        "forbidden",
        "bad request",
        "badrequest",
        "validation",
        "invalid",
        "config",
        "configuration",
        "authentication",
        "permission",
    )
    if any(token in text for token in non_retryable_tokens):
        return False

    retryable_tokens = (
        "timeout",
        "timed out",
        "transient",
        "temporary",
        "network",
        "connection",
        "rate limit",
        "ratelimit",
        "too many requests",
        "429",
        "500",
        "502",
        "503",
        "504",
        "unavailable",
        "reset by peer",
        "econn",
    )
    return any(token in text for token in retryable_tokens)


def _extract_status_code(exc: Exception) -> int | None:
    for key in ("status_code", "status", "http_status"):
        parsed = _coerce_status_code(getattr(exc, key, None))
        if parsed is not None:
            return parsed

    response = getattr(exc, "response", None)
    if response is None:
        return None

    for key in ("status_code", "status"):
        parsed = _coerce_status_code(getattr(response, key, None))
        if parsed is not None:
            return parsed
    return None


def _coerce_status_code(value: object) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return None
    try:
        return int(str(value).strip())
    except (TypeError, ValueError):
        return None
