from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict

from attractor.dsl.models import DotNode

from .base import Handler


SHAPE_TO_TYPE = {
    "Mdiamond": "start",
    "Msquare": "exit",
    "box": "codergen",
    "hexagon": "wait.human",
    "diamond": "conditional",
    "component": "parallel",
    "tripleoctagon": "parallel.fan_in",
    "parallelogram": "tool",
    "house": "stack.manager_loop",
}


@dataclass
class HandlerRegistry:
    handlers: Dict[str, Handler] = field(default_factory=dict)
    default_handler_type: str = "codergen"

    def register(self, handler_type: str, handler: Handler) -> None:
        self.handlers[handler_type] = handler

    def get(self, handler_type: str) -> Handler:
        if handler_type not in self.handlers:
            raise KeyError(f"No handler registered for type '{handler_type}'")
        return self.handlers[handler_type]

    def resolve_handler_type(self, node: DotNode) -> str:
        explicit = node.attrs.get("type")
        if explicit:
            explicit_value = str(explicit.value).strip()
            if explicit_value and explicit_value in self.handlers:
                return explicit_value

        shape = node.attrs.get("shape")
        if shape:
            mapped = SHAPE_TO_TYPE.get(str(shape.value), None)
            if mapped and mapped in self.handlers:
                return mapped

        if self.default_handler_type in self.handlers:
            return self.default_handler_type

        raise KeyError(f"No handler registered for default type '{self.default_handler_type}'")

    def resolve(self, node: DotNode) -> Handler:
        handler_type = self.resolve_handler_type(node)
        return self.get(handler_type)
