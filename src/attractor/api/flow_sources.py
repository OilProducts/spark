from __future__ import annotations

from pathlib import Path, PurePosixPath
from typing import Callable

from fastapi import HTTPException

from attractor.dsl import format_dot, normalize_graph, parse_dot
from attractor.dsl.models import DotAttribute, DotValueType


def ensure_flows_dir(flows_dir: Path) -> Path:
    flows_dir.mkdir(parents=True, exist_ok=True)
    return flows_dir


def normalize_flow_name(flow_name: str) -> str:
    raw_name = flow_name.strip().replace("\\", "/")
    if not raw_name:
        raise HTTPException(status_code=400, detail="Flow name is required.")
    if raw_name.endswith("/"):
        raise HTTPException(status_code=400, detail="Flow name must reference a file.")

    candidate = PurePosixPath(raw_name)
    if candidate.is_absolute() or ".." in candidate.parts or "." in candidate.parts or not candidate.parts:
        raise HTTPException(status_code=400, detail="Flow name must be a relative path inside flows_dir.")

    leaf_name = candidate.name
    if not leaf_name:
        raise HTTPException(status_code=400, detail="Flow name must reference a file.")
    if not leaf_name.endswith(".dot"):
        leaf_name = f"{leaf_name}.dot"

    return PurePosixPath(*candidate.parts[:-1], leaf_name).as_posix()


def resolve_flow_path(flows_dir: Path, flow_name: str) -> Path:
    normalized_name = normalize_flow_name(flow_name)
    return ensure_flows_dir(flows_dir) / Path(*PurePosixPath(normalized_name).parts)


def flow_name_from_path(flows_dir: Path, flow_path: Path) -> str:
    root = ensure_flows_dir(flows_dir)
    return flow_path.relative_to(root).as_posix()


def inject_pipeline_goal(flow_content: str, goal: str) -> str:
    graph = parse_dot(flow_content)
    existing = graph.graph_attrs.get("goal")
    graph.graph_attrs["goal"] = DotAttribute(
        key="goal",
        value=goal,
        value_type=DotValueType.STRING,
        line=existing.line if existing is not None else 0,
    )
    return format_dot(graph)


def load_flow_content(flows_dir: Path, flow_source: str) -> str:
    flow_path = resolve_flow_path(flows_dir, flow_source)
    if not flow_path.exists():
        raise HTTPException(status_code=404, detail=f"Flow not found: {flow_path.name}")
    return flow_path.read_text(encoding="utf-8")


def load_execution_planning_flow_content(flows_dir: Path, flow_source: str, prompt: str) -> str:
    return inject_pipeline_goal(load_flow_content(flows_dir, flow_source), prompt)


def semantic_signature(dot_content: str, build_transform_pipeline: Callable[[], object]) -> str:
    graph = build_transform_pipeline().apply(parse_dot(dot_content))
    normalized = normalize_graph(graph)
    normalized.graph_id = "__semantic__"
    return format_dot(normalized)
