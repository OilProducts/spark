from __future__ import annotations

from typing import Protocol

from attractor.dsl.models import DotGraph


class Transform(Protocol):
    def apply(self, graph: DotGraph) -> DotGraph:
        ...
