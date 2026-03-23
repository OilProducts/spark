from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tomllib

from fastapi import HTTPException

from attractor.api.flow_sources import (
    ensure_flows_dir,
    flow_name_from_path,
    normalize_flow_name as _normalize_flow_name_impl,
    resolve_flow_path,
)
from attractor.dsl import parse_dot
from attractor.dsl.models import DotGraph, DotNode


FLOW_CATALOG_FILE_NAME = "flow-catalog.toml"
LAUNCH_POLICY_AGENT_REQUESTABLE = "agent_requestable"
LAUNCH_POLICY_TRIGGER_ONLY = "trigger_only"
LAUNCH_POLICY_DISABLED = "disabled"
ALLOWED_LAUNCH_POLICIES = {
    LAUNCH_POLICY_AGENT_REQUESTABLE,
    LAUNCH_POLICY_TRIGGER_ONLY,
    LAUNCH_POLICY_DISABLED,
}


@dataclass(frozen=True)
class FlowLaunchPolicyState:
    name: str
    launch_policy: str | None
    effective_launch_policy: str


@dataclass(frozen=True)
class FlowGraphFeatures:
    has_human_gate: bool
    has_manager_loop: bool


@dataclass(frozen=True)
class FlowSummary:
    name: str
    title: str
    description: str
    launch_policy: str | None
    effective_launch_policy: str
    graph_label: str
    graph_goal: str


@dataclass(frozen=True)
class FlowDescription(FlowSummary):
    node_count: int
    edge_count: int
    features: FlowGraphFeatures


def flow_catalog_path(config_dir: Path) -> Path:
    config_dir.mkdir(parents=True, exist_ok=True)
    return config_dir / FLOW_CATALOG_FILE_NAME


def load_flow_catalog(config_dir: Path) -> dict[str, str]:
    path = flow_catalog_path(config_dir)
    if not path.exists():
        return {}
    try:
        payload = tomllib.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise RuntimeError(f"Invalid flow catalog file: {path}") from exc
    flows_section = payload.get("flows")
    if flows_section is None:
        return {}
    if not isinstance(flows_section, dict):
        raise RuntimeError(f"Flow catalog file is missing a valid [flows] section: {path}")

    catalog: dict[str, str] = {}
    for raw_flow_name, raw_entry in flows_section.items():
        if not isinstance(raw_flow_name, str):
            raise RuntimeError(f"Flow catalog file contains a non-string flow name: {path}")
        if not isinstance(raw_entry, dict):
            raise RuntimeError(f"Flow catalog entry for {raw_flow_name!r} must be a table: {path}")
        raw_policy = raw_entry.get("launch_policy")
        if raw_policy is None:
            continue
        if not isinstance(raw_policy, str):
            raise RuntimeError(f"Flow catalog entry for {raw_flow_name!r} must define launch_policy as a string: {path}")
        flow_name = normalize_flow_name(raw_flow_name)
        launch_policy = normalize_launch_policy(raw_policy)
        catalog[flow_name] = launch_policy
    return catalog


def write_flow_catalog(config_dir: Path, catalog: dict[str, str]) -> Path:
    path = flow_catalog_path(config_dir)
    lines: list[str] = []
    for flow_name in sorted(catalog.keys()):
        launch_policy = normalize_launch_policy(catalog[flow_name])
        lines.extend(
            [
                f'[flows.{_toml_string(flow_name)}]',
                f"launch_policy = {_toml_string(launch_policy)}",
                "",
            ]
        )
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def read_flow_launch_policy(config_dir: Path, flow_name: str) -> FlowLaunchPolicyState:
    normalized_flow_name = normalize_flow_name(flow_name)
    catalog = load_flow_catalog(config_dir)
    launch_policy = catalog.get(normalized_flow_name)
    return FlowLaunchPolicyState(
        name=normalized_flow_name,
        launch_policy=launch_policy,
        effective_launch_policy=launch_policy or LAUNCH_POLICY_DISABLED,
    )


def set_flow_launch_policy(config_dir: Path, flow_name: str, launch_policy: str) -> FlowLaunchPolicyState:
    normalized_flow_name = normalize_flow_name(flow_name)
    normalized_launch_policy = normalize_launch_policy(launch_policy)
    catalog = load_flow_catalog(config_dir)
    catalog[normalized_flow_name] = normalized_launch_policy
    write_flow_catalog(config_dir, catalog)
    return FlowLaunchPolicyState(
        name=normalized_flow_name,
        launch_policy=normalized_launch_policy,
        effective_launch_policy=normalized_launch_policy,
    )


def list_flow_summaries(flows_dir: Path, config_dir: Path) -> list[FlowSummary]:
    catalog = load_flow_catalog(config_dir)
    summaries: list[FlowSummary] = []
    for flow_path in _iter_flow_paths(flows_dir):
        flow_name = flow_name_from_path(flows_dir, flow_path)
        summaries.append(_build_flow_summary(flow_path, flow_name, catalog.get(flow_name)))
    return summaries


def read_flow_summary(flows_dir: Path, config_dir: Path, flow_name: str) -> FlowSummary:
    flow_path = _resolve_existing_flow_path(flows_dir, flow_name)
    catalog = load_flow_catalog(config_dir)
    normalized_flow_name = flow_name_from_path(flows_dir, flow_path)
    return _build_flow_summary(flow_path, normalized_flow_name, catalog.get(normalized_flow_name))


def read_flow_description(flows_dir: Path, config_dir: Path, flow_name: str) -> FlowDescription:
    flow_path = _resolve_existing_flow_path(flows_dir, flow_name)
    raw_content = flow_path.read_text(encoding="utf-8")
    try:
        graph = parse_dot(raw_content)
    except Exception as exc:
        raise RuntimeError(f"Invalid flow file: {flow_name_from_path(flows_dir, flow_path)}") from exc
    normalized_flow_name = flow_name_from_path(flows_dir, flow_path)
    policy_state = read_flow_launch_policy(config_dir, normalized_flow_name)
    title, description, graph_label, graph_goal = _resolve_flow_metadata(graph, normalized_flow_name)
    return FlowDescription(
        name=normalized_flow_name,
        title=title,
        description=description,
        launch_policy=policy_state.launch_policy,
        effective_launch_policy=policy_state.effective_launch_policy,
        graph_label=graph_label,
        graph_goal=graph_goal,
        node_count=len(graph.nodes),
        edge_count=len(graph.edges),
        features=FlowGraphFeatures(
            has_human_gate=any(_is_human_gate(node) for node in graph.nodes.values()),
            has_manager_loop=any(_is_manager_loop(node) for node in graph.nodes.values()),
        ),
    )


def read_flow_raw(flows_dir: Path, flow_name: str) -> tuple[str, str]:
    flow_path = _resolve_existing_flow_path(flows_dir, flow_name)
    return flow_name_from_path(flows_dir, flow_path), flow_path.read_text(encoding="utf-8")


def normalize_flow_name(flow_name: str) -> str:
    try:
        return _normalize_flow_name_impl(flow_name)
    except HTTPException as exc:
        raise ValueError(str(exc.detail)) from exc


def normalize_launch_policy(launch_policy: str) -> str:
    normalized = launch_policy.strip().lower()
    if normalized not in ALLOWED_LAUNCH_POLICIES:
        allowed = ", ".join(sorted(ALLOWED_LAUNCH_POLICIES))
        raise ValueError(f"Launch policy must be one of: {allowed}")
    return normalized


def _resolve_existing_flow_path(flows_dir: Path, flow_name: str) -> Path:
    flow_path = resolve_flow_path(flows_dir, flow_name)
    if not flow_path.exists():
        raise FileNotFoundError(normalize_flow_name(flow_name))
    return flow_path


def _build_flow_summary(flow_path: Path, flow_name: str, launch_policy: str | None) -> FlowSummary:
    graph_label = ""
    graph_goal = ""
    title = flow_path.stem
    description = ""
    try:
        graph = parse_dot(flow_path.read_text(encoding="utf-8"))
        title, description, graph_label, graph_goal = _resolve_flow_metadata(graph, flow_name)
    except Exception:
        pass
    return FlowSummary(
        name=flow_name,
        title=title,
        description=description,
        launch_policy=launch_policy,
        effective_launch_policy=launch_policy or LAUNCH_POLICY_DISABLED,
        graph_label=graph_label,
        graph_goal=graph_goal,
    )


def _iter_flow_paths(flows_dir: Path) -> list[Path]:
    root = ensure_flows_dir(flows_dir)
    return sorted(
        (path for path in root.rglob("*.dot") if path.is_file()),
        key=lambda path: flow_name_from_path(root, path),
    )


def _resolve_flow_metadata(graph: DotGraph, flow_name: str) -> tuple[str, str, str, str]:
    graph_label = _graph_attr_string(graph, "label")
    graph_goal = _graph_attr_string(graph, "goal")
    spark_title = _graph_attr_string(graph, "spark.title")
    spark_description = _graph_attr_string(graph, "spark.description")
    title = spark_title or graph_label or Path(flow_name).stem
    description = spark_description or graph_goal or ""
    return title, description, graph_label, graph_goal


def _graph_attr_string(graph: DotGraph, key: str) -> str:
    attr = graph.graph_attrs.get(key)
    if attr is None:
        return ""
    return str(attr.value).strip()


def _node_attr_string(node: DotNode, key: str) -> str:
    attr = node.attrs.get(key)
    if attr is None:
        return ""
    return str(attr.value).strip()


def _is_human_gate(node: DotNode) -> bool:
    node_type = _node_attr_string(node, "type")
    node_shape = _node_attr_string(node, "shape")
    return node_type == "wait.human" or node_shape == "hexagon"


def _is_manager_loop(node: DotNode) -> bool:
    node_type = _node_attr_string(node, "type")
    node_shape = _node_attr_string(node, "shape")
    return node_type == "stack.manager_loop" or node_shape == "house"


def _toml_string(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace("\"", "\\\"")
    return f'"{escaped}"'
