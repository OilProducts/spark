from __future__ import annotations

from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import nullcontext
import math
from typing import Any, Dict, List, Tuple

from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus

from ..base import HandlerRuntime


SUCCESS_STATUSES = {"success", "paused"}
SUPPORTED_JOIN_POLICIES = {"wait_all", "k_of_n", "first_success", "quorum"}
SUPPORTED_ERROR_POLICIES = {"fail_fast", "continue", "ignore"}


class ParallelHandler:
    def execute(self, runtime: HandlerRuntime) -> Outcome:
        if not runtime.outgoing_edges:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="parallel node has no outgoing edges")

        join_policy = _attr_str(runtime.node_attrs, "join_policy", "wait_all")
        if join_policy not in SUPPORTED_JOIN_POLICIES:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=f"unsupported join_policy: {join_policy}")
        error_policy = _attr_str(runtime.node_attrs, "error_policy", "continue")
        if error_policy not in SUPPORTED_ERROR_POLICIES:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=f"unsupported error_policy: {error_policy}")
        max_parallel = _attr_int(runtime.node_attrs, "max_parallel", 4)
        if max_parallel < 1:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="max_parallel must be >= 1")

        fan_in_nodes = _fan_in_nodes(runtime.graph)

        def run_branch(target: str, base_context: Context) -> Tuple[str, Dict[str, Any]]:
            branch_context = base_context.clone()
            executor = PipelineExecutor(
                runtime.graph,
                runtime.runner,
                logs_root=str(runtime.logs_root) if runtime.logs_root is not None else None,
            )
            result = executor.run_from(
                target,
                branch_context,
                stop_nodes=fan_in_nodes,
            )
            payload = {
                "id": target,
                "status": result.status,
                "current_node": result.current_node,
                "completed_nodes": result.completed_nodes,
                "context": result.context,
                "node_outcomes": {k: v.status.value for k, v in result.node_outcomes.items()},
                "failure_reason": result.failure_reason,
            }
            return target, payload

        results: List[Dict[str, Any]] = []
        allow_concurrency = getattr(runtime.runner, "allow_concurrency", None)
        concurrency_scope = allow_concurrency() if callable(allow_concurrency) else nullcontext()
        with concurrency_scope:
            with ThreadPoolExecutor(max_workers=max_parallel) as pool:
                pending = {}
                edge_iter = iter(runtime.outgoing_edges)

                def submit_next() -> bool:
                    try:
                        edge = next(edge_iter)
                    except StopIteration:
                        return False
                    future = pool.submit(run_branch, edge.target, runtime.context)
                    pending[future] = edge.target
                    return True

                for _ in range(max_parallel):
                    if not submit_next():
                        break

                terminated_early = False
                while pending:
                    completed, _ = wait(set(pending.keys()), return_when=FIRST_COMPLETED)
                    for future in completed:
                        pending.pop(future, None)
                        _, payload = future.result()
                        results.append(payload)

                        if error_policy == "fail_fast" and payload["status"] == "fail":
                            terminated_early = True

                        if join_policy == "first_success" and payload["status"] in SUCCESS_STATUSES:
                            terminated_early = True

                    if terminated_early:
                        for remaining in pending:
                            remaining.cancel()
                        break

                    while len(pending) < max_parallel and submit_next():
                        pass

        results_for_policy = list(results)
        if error_policy == "ignore":
            results_for_policy = [r for r in results_for_policy if r["status"] in SUCCESS_STATUSES]

        success_count = sum(1 for r in results_for_policy if r["status"] in SUCCESS_STATUSES)
        fail_count = sum(1 for r in results_for_policy if r["status"] == "fail")

        outcome_status = OutcomeStatus.SUCCESS
        if join_policy == "wait_all":
            outcome_status = OutcomeStatus.SUCCESS if fail_count == 0 else OutcomeStatus.PARTIAL_SUCCESS
        elif join_policy == "first_success":
            outcome_status = OutcomeStatus.SUCCESS if success_count > 0 else OutcomeStatus.FAIL
        elif join_policy == "k_of_n":
            required = max(1, _attr_int(runtime.node_attrs, "join_k", len(results_for_policy)))
            outcome_status = OutcomeStatus.SUCCESS if success_count >= required else OutcomeStatus.FAIL
        elif join_policy == "quorum":
            quorum = _attr_float(runtime.node_attrs, "join_quorum", 0.5)
            required = max(1, math.ceil(len(results_for_policy) * quorum))
            outcome_status = OutcomeStatus.SUCCESS if success_count >= required else OutcomeStatus.FAIL

        return Outcome(
            status=outcome_status,
            context_updates={"parallel.results": results_for_policy if error_policy == "ignore" else results},
            notes="parallel fan-out completed",
        )


def _fan_in_nodes(graph) -> set[str]:
    fan_in = set()
    for node_id, node in graph.nodes.items():
        type_attr = node.attrs.get("type")
        if type_attr and str(type_attr.value).strip() == "parallel.fan_in":
            fan_in.add(node_id)
            continue
        shape_attr = node.attrs.get("shape")
        if shape_attr and str(shape_attr.value) == "tripleoctagon":
            fan_in.add(node_id)
    return fan_in


def _attr_str(attrs: Dict[str, Any], key: str, default: str) -> str:
    attr = attrs.get(key)
    if not attr:
        return default
    return str(attr.value)


def _attr_int(attrs: Dict[str, Any], key: str, default: int) -> int:
    attr = attrs.get(key)
    if not attr:
        return default
    try:
        return int(str(attr.value))
    except (TypeError, ValueError):
        return default


def _attr_float(attrs: Dict[str, Any], key: str, default: float) -> float:
    attr = attrs.get(key)
    if not attr:
        return default
    try:
        return float(str(attr.value))
    except (TypeError, ValueError):
        return default
