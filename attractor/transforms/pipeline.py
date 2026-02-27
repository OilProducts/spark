from __future__ import annotations

import copy
from dataclasses import dataclass, field
from typing import List

from attractor.dsl.models import DotGraph

from .base import Transform


@dataclass
class TransformPipeline:
    transforms: List[Transform] = field(default_factory=list)

    def register(self, transform: Transform) -> None:
        self.transforms.append(transform)

    def apply(self, graph: DotGraph) -> DotGraph:
        cur = copy.deepcopy(graph)
        for transform in self.transforms:
            # Apply a per-run clone when possible so stateful transform instances
            # do not leak mutable state across independent pipeline runs.
            try:
                transform_instance = copy.deepcopy(transform)
            except Exception:
                transform_instance = transform
            apply_fn = getattr(transform_instance, "apply", None)
            if callable(apply_fn):
                cur = apply_fn(cur)
                continue
            transform_fn = getattr(transform_instance, "transform", None)
            if callable(transform_fn):
                cur = transform_fn(cur)
                continue
            raise TypeError("Transform must implement apply(graph) or transform(graph)")
        return cur
