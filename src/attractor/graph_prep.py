from __future__ import annotations

from typing import Iterable, List, Mapping

from attractor.dsl import Diagnostic, parse_dot, validate_graph
from attractor.dsl.formatter import format_dot
from attractor.dsl.models import DotAttribute, DotGraph


DEFAULT_MAX_RETRIES_KEY = "default_max_retries"


def canonicalize_graph_source(source: str) -> str:
    graph = parse_dot(source)
    return format_dot(graph)


def build_transform_pipeline(extra_transforms: Iterable[object] = ()) -> object:
    # Lazy import avoids transform package initialization during module import.
    from attractor.transforms.pipeline import TransformPipeline
    from attractor.transforms.stylesheet import ModelStylesheetTransform
    from attractor.transforms.variables import GoalVariableTransform

    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    for transform in extra_transforms:
        pipeline.register(transform)
    return pipeline


def apply_graph_transforms(graph: DotGraph, extra_transforms: Iterable[object] = ()) -> DotGraph:
    return build_transform_pipeline(extra_transforms).apply(graph)


def prepare_graph(graph: DotGraph, extra_transforms: Iterable[object] = ()) -> tuple[DotGraph, List[Diagnostic]]:
    transformed = apply_graph_transforms(graph, extra_transforms)
    diagnostics = validate_graph(transformed)
    return transformed, diagnostics


def parse_prepare_graph(source: str, extra_transforms: Iterable[object] = ()) -> tuple[DotGraph, List[Diagnostic]]:
    graph = parse_dot(source)
    return prepare_graph(graph, extra_transforms)


def resolve_default_max_retries_attr(attrs: Mapping[str, DotAttribute]) -> DotAttribute | None:
    return attrs.get(DEFAULT_MAX_RETRIES_KEY)


def resolve_default_max_retries_value(attrs: Mapping[str, DotAttribute], default: int = 0) -> int:
    attr = resolve_default_max_retries_attr(attrs)
    if attr is None:
        return default
    value = attr.value
    if isinstance(value, int):
        return value
    try:
        return int(str(value))
    except (TypeError, ValueError):
        return default
