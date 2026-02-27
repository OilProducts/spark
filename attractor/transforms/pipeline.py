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
            cur = transform.apply(cur)
        return cur
