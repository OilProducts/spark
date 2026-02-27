from __future__ import annotations

import os
from typing import Any, Callable

from attractor.dsl.models import DotAttribute, DotGraph, DotValueType


_DefaultFactory = Callable[[str], Any]


def _static(value: Any) -> _DefaultFactory:
    return lambda _node_id: value


GRAPH_DEFAULTS: tuple[tuple[str, DotValueType, _DefaultFactory], ...] = (
    ("goal", DotValueType.STRING, _static("")),
    ("label", DotValueType.STRING, _static("")),
    ("model_stylesheet", DotValueType.STRING, _static("")),
    ("default_max_retry", DotValueType.INTEGER, _static(50)),
    ("retry_target", DotValueType.STRING, _static("")),
    ("fallback_retry_target", DotValueType.STRING, _static("")),
    ("default_fidelity", DotValueType.STRING, _static("")),
    ("stack.child_dotfile", DotValueType.STRING, _static("")),
    ("stack.child_workdir", DotValueType.STRING, lambda _graph_id: os.getcwd()),
    ("tool_hooks.pre", DotValueType.STRING, _static("")),
    ("tool_hooks.post", DotValueType.STRING, _static("")),
)

NODE_DEFAULTS: tuple[tuple[str, DotValueType, _DefaultFactory], ...] = (
    ("label", DotValueType.STRING, lambda node_id: node_id),
    ("shape", DotValueType.STRING, _static("box")),
    ("type", DotValueType.STRING, _static("")),
    ("prompt", DotValueType.STRING, _static("")),
    ("max_retries", DotValueType.INTEGER, _static(0)),
    ("goal_gate", DotValueType.BOOLEAN, _static(False)),
    ("retry_target", DotValueType.STRING, _static("")),
    ("fallback_retry_target", DotValueType.STRING, _static("")),
    ("fidelity", DotValueType.STRING, _static("")),
    ("thread_id", DotValueType.STRING, _static("")),
    ("class", DotValueType.STRING, _static("")),
    ("timeout", DotValueType.DURATION, _static(None)),
    ("llm_model", DotValueType.STRING, _static("")),
    ("llm_provider", DotValueType.STRING, _static("")),
    ("reasoning_effort", DotValueType.STRING, _static("high")),
    ("auto_status", DotValueType.BOOLEAN, _static(False)),
    ("allow_partial", DotValueType.BOOLEAN, _static(False)),
)

EDGE_DEFAULTS: tuple[tuple[str, DotValueType, _DefaultFactory], ...] = (
    ("label", DotValueType.STRING, _static("")),
    ("condition", DotValueType.STRING, _static("")),
    ("weight", DotValueType.INTEGER, _static(0)),
    ("loop_restart", DotValueType.BOOLEAN, _static(False)),
)


class AttributeDefaultsTransform:
    def apply(self, graph: DotGraph) -> DotGraph:
        for key, value_type, factory in GRAPH_DEFAULTS:
            _set_default(graph.graph_attrs, key, value_type, factory(""))

        for node in graph.nodes.values():
            for key, value_type, factory in NODE_DEFAULTS:
                _set_default(node.attrs, key, value_type, factory(node.node_id))

        for edge in graph.edges:
            for key, value_type, factory in EDGE_DEFAULTS:
                _set_default(edge.attrs, key, value_type, factory(edge.target))

        return graph


def _set_default(attrs: dict[str, DotAttribute], key: str, value_type: DotValueType, value: Any) -> None:
    if key in attrs:
        return
    attrs[key] = DotAttribute(
        key=key,
        value=value,
        value_type=value_type,
        line=0,
    )
