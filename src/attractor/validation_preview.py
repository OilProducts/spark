from __future__ import annotations

from collections.abc import Iterable
from typing import Any

from attractor.dsl import Diagnostic, DiagnosticSeverity, DotGraph, DotParseError, parse_dot
from attractor.graph_prep import prepare_graph


def diagnostic_payload(diagnostic: Diagnostic) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "rule": diagnostic.rule_id,
        "rule_id": diagnostic.rule_id,
        "severity": diagnostic.severity.value,
        "message": diagnostic.message,
        "line": diagnostic.line,
        "node": diagnostic.node_id,
        "node_id": diagnostic.node_id,
    }
    if diagnostic.edge is not None:
        payload["edge"] = list(diagnostic.edge)
    if diagnostic.fix is not None:
        payload["fix"] = diagnostic.fix
    return payload


def preview_dot_source(
    dot_source: str,
    *,
    extra_transforms: Iterable[object] = (),
) -> tuple[DotGraph | None, dict[str, Any]]:
    try:
        graph = parse_dot(dot_source)
    except DotParseError as exc:
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
            "node_id": None,
        }
        return None, {
            "status": "parse_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    transformed_graph, diagnostics = prepare_graph(graph, extra_transforms=extra_transforms)
    errors = [diagnostic for diagnostic in diagnostics if diagnostic.severity == DiagnosticSeverity.ERROR]
    return transformed_graph, {
        "status": "ok" if not errors else "validation_error",
        "diagnostics": [diagnostic_payload(diagnostic) for diagnostic in diagnostics],
        "errors": [diagnostic_payload(diagnostic) for diagnostic in errors],
    }
