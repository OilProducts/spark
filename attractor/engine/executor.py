from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path
from typing import Callable, Dict, List, Optional, Tuple

from attractor.dsl.models import DotGraph

from .checkpoint import Checkpoint, load_checkpoint, save_checkpoint
from .context import Context
from .outcome import Outcome
from .routing import select_next_edge


RunnerFn = Callable[[str, str, Context], Outcome]
ControlFn = Callable[[], Optional[str]]
NODE_OUTCOMES_KEY = "_attractor.node_outcomes"


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
    ):
        self.graph = graph
        self.runner = runner
        self.logs_root = Path(logs_root) if logs_root else None
        self.checkpoint_path = Path(checkpoint_file) if checkpoint_file else None
        self.control = control
        self._shape_start_nodes = self._node_ids_for_shape("Mdiamond")
        self._shape_exit_nodes = self._node_ids_for_shape("Msquare")

    def run(
        self,
        context: Optional[Context] = None,
        *,
        resume: bool = False,
        max_steps: Optional[int] = None,
    ) -> PipelineResult:
        ctx = context or Context()
        completed: List[str] = []
        outcomes: Dict[str, Outcome] = {}
        retry_counts: Dict[str, int] = {}

        current = self._resolve_start_node()
        route_trace: List[str] = [current]
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
                    restored_context = dict(checkpoint.context)
                    restored_context.update(ctx.values)
                    ctx = Context(values=restored_context)

        steps = 0
        while True:
            action = self._poll_control()
            if action == "abort":
                self._save_checkpoint(
                    current_node=current,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
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
                self._save_checkpoint(
                    current_node=current,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
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
                gates_ok, failed_gate_node = self._check_goal_gates(ctx)
                if gates_ok:
                    self._save_checkpoint(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
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
                    route_trace.append(current)
                    self._save_checkpoint(
                        current_node=current,
                        completed_nodes=completed,
                        context=ctx,
                        retry_counts=retry_counts,
                    )
                    continue

                return PipelineResult(
                    status="fail",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

            node = self.graph.nodes[current]
            prompt = self._prompt_for_node(node.node_id)
            outcome = self.runner(node.node_id, prompt, ctx)
            outcomes[node.node_id] = outcome

            ctx.set("outcome", outcome.status.value)
            if outcome.preferred_label:
                ctx.set("preferred_label", outcome.preferred_label)
            if outcome.context_updates:
                ctx.merge_updates(outcome.context_updates)
            self._remember_node_outcome(ctx, node.node_id, outcome.status.value)

            self._write_stage_artifacts(node.node_id, prompt, outcome)

            max_retries = self._max_retries_for_node(node.node_id)
            retries_so_far = retry_counts.get(node.node_id, 0)
            if self._should_retry(outcome, retries_so_far, max_retries):
                retry_counts[node.node_id] = retries_so_far + 1
                self._save_checkpoint(
                    current_node=node.node_id,
                    completed_nodes=completed,
                    context=ctx,
                    retry_counts=retry_counts,
                )
                continue

            completed.append(node.node_id)
            outgoing = [edge for edge in self.graph.edges if edge.source == node.node_id]
            next_edge = select_next_edge(outgoing, outcome, ctx)
            if not next_edge and outcome.status.value == "fail":
                route = self._resolve_failure_retry_target(node.node_id)
                if route:
                    next_edge = _SyntheticEdge(route)
            if not next_edge:
                raise RuntimeError(
                    f"Stage '{node.node_id}' has no eligible outgoing edge"
                )

            current = next_edge.target
            route_trace.append(current)
            self._save_checkpoint(
                current_node=current,
                completed_nodes=completed,
                context=ctx,
                retry_counts=retry_counts,
            )

            steps += 1
            if max_steps is not None and steps >= max_steps:
                return PipelineResult(
                    status="paused",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

    def run_from(
        self,
        start_node: str,
        context: Optional[Context] = None,
        *,
        max_steps: Optional[int] = None,
        stop_nodes: Optional[set[str]] = None,
    ) -> PipelineResult:
        ctx = context or Context()
        completed: List[str] = []
        outcomes: Dict[str, Outcome] = {}
        retry_counts: Dict[str, int] = {}
        current = start_node
        route_trace: List[str] = [current]
        steps = 0
        stop_nodes = set(stop_nodes or [])

        while True:
            action = self._poll_control()
            if action == "abort":
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
                return PipelineResult(
                    status="paused",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

            if current in stop_nodes:
                return PipelineResult(
                    status="success",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

            if self._is_exit_node(current):
                return PipelineResult(
                    status="success",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

            node = self.graph.nodes[current]
            prompt = self._prompt_for_node(node.node_id)
            outcome = self.runner(node.node_id, prompt, ctx)
            outcomes[node.node_id] = outcome

            ctx.set("outcome", outcome.status.value)
            if outcome.preferred_label:
                ctx.set("preferred_label", outcome.preferred_label)
            if outcome.context_updates:
                ctx.merge_updates(outcome.context_updates)
            self._remember_node_outcome(ctx, node.node_id, outcome.status.value)

            self._write_stage_artifacts(node.node_id, prompt, outcome)

            max_retries = self._max_retries_for_node(node.node_id)
            retries_so_far = retry_counts.get(node.node_id, 0)
            if self._should_retry(outcome, retries_so_far, max_retries):
                retry_counts[node.node_id] = retries_so_far + 1
                continue

            completed.append(node.node_id)
            outgoing = [edge for edge in self.graph.edges if edge.source == node.node_id]
            next_edge = select_next_edge(outgoing, outcome, ctx)
            if not next_edge and outcome.status.value == "fail":
                route = self._resolve_failure_retry_target(node.node_id)
                if route:
                    next_edge = _SyntheticEdge(route)
            if not next_edge:
                return PipelineResult(
                    status="fail",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                    failure_reason=f"Stage '{node.node_id}' has no eligible outgoing edge",
                )

            current = next_edge.target
            route_trace.append(current)

            steps += 1
            if max_steps is not None and steps >= max_steps:
                return PipelineResult(
                    status="paused",
                    current_node=current,
                    completed_nodes=completed,
                    context=dict(ctx.values),
                    node_outcomes=outcomes,
                    route_trace=route_trace,
                )

    def _save_checkpoint(
        self,
        current_node: str,
        completed_nodes: List[str],
        context: Context,
        retry_counts: Dict[str, int],
    ) -> None:
        if not self.checkpoint_path:
            return
        checkpoint = Checkpoint(
            current_node=current_node,
            completed_nodes=list(completed_nodes),
            context=dict(context.values),
            retry_counts=dict(retry_counts),
        )
        save_checkpoint(self.checkpoint_path, checkpoint)

    def _poll_control(self) -> Optional[str]:
        if not self.control:
            return None
        try:
            return self.control()
        except Exception:
            return None

    def _write_stage_artifacts(self, node_id: str, prompt: str, outcome: Outcome) -> None:
        if not self.logs_root:
            return

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
        }
        with (stage_dir / "status.json").open("w", encoding="utf-8") as f:
            json.dump(status_payload, f, indent=2, sort_keys=True)

    def _resolve_start_node(self) -> str:
        starts = [
            node.node_id
            for node in self.graph.nodes.values()
            if self._is_start_node(node.node_id)
        ]
        if len(starts) != 1:
            raise RuntimeError(f"Expected exactly one start node, found {len(starts)}")
        return starts[0]

    def _is_start_node(self, node_id: str) -> bool:
        if self._shape_start_nodes:
            return node_id in self._shape_start_nodes
        return node_id in {"start", "Start"}

    def _is_exit_node(self, node_id: str) -> bool:
        if self._shape_exit_nodes:
            return node_id in self._shape_exit_nodes
        return node_id in {"exit", "end", "Exit", "End"}

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
        if prompt_attr:
            return str(prompt_attr.value)
        label_attr = node.attrs.get("label")
        if label_attr:
            return str(label_attr.value)
        return node_id

    def _max_retries_for_node(self, node_id: str) -> int:
        node = self.graph.nodes[node_id]
        node_attr = node.attrs.get("max_retries")
        if node_attr:
            return _to_int(node_attr.value, 0)

        graph_attr = self.graph.graph_attrs.get("default_max_retry")
        if graph_attr:
            return _to_int(graph_attr.value, 50)
        return 50

    def _should_retry(self, outcome: Outcome, retries_so_far: int, max_retries: int) -> bool:
        if retries_so_far >= max_retries:
            return False
        if outcome.status.value == "retry":
            return True
        if outcome.status.value == "fail":
            reason = (outcome.failure_reason or "").lower()
            if reason == "":
                return False
            retryable_signals = (
                "timeout",
                "timed out",
                "rate limit",
                "429",
                "5xx",
                "network",
                "temporar",
                "retryable",
                "unavailable",
            )
            return any(signal in reason for signal in retryable_signals)
        return False

    def _resolve_failure_retry_target(self, node_id: str) -> str:
        node = self.graph.nodes[node_id]
        for key in ("retry_target", "fallback_retry_target"):
            attr = node.attrs.get(key)
            if attr:
                target = str(attr.value)
                if target in self.graph.nodes:
                    return target

        for key in ("retry_target", "fallback_retry_target"):
            attr = self.graph.graph_attrs.get(key)
            if attr:
                target = str(attr.value)
                if target in self.graph.nodes:
                    return target
        return ""

    def _remember_node_outcome(self, context: Context, node_id: str, status: str) -> None:
        stored = context.get(NODE_OUTCOMES_KEY, {})
        if not isinstance(stored, dict):
            stored = {}
        stored = dict(stored)
        stored[node_id] = status
        context.set(NODE_OUTCOMES_KEY, stored)

    def _check_goal_gates(self, context: Context) -> Tuple[bool, str]:
        statuses = context.get(NODE_OUTCOMES_KEY, {})
        if not isinstance(statuses, dict):
            statuses = {}

        for node_id, node in self.graph.nodes.items():
            goal_gate_attr = node.attrs.get("goal_gate")
            if not goal_gate_attr:
                continue
            if not _to_bool(goal_gate_attr.value):
                continue

            if node_id not in statuses:
                continue
            if statuses.get(node_id) != "success":
                return False, node_id

        return True, ""

    def _resolve_goal_gate_retry_target(self, failed_gate_node: str) -> str:
        if not failed_gate_node:
            return ""
        return self._resolve_failure_retry_target(failed_gate_node)


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
