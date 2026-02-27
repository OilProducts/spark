"""DOT DSL parsing and validation."""

from .models import Diagnostic, DiagnosticSeverity, DotGraph
from .formatter import canonicalize_dot, format_dot
from .parser import DotParseError, normalize_graph, parse_dot
from .validator import ValidationError, validate, validate_graph, validate_or_raise

__all__ = [
    "Diagnostic",
    "DiagnosticSeverity",
    "DotGraph",
    "DotParseError",
    "canonicalize_dot",
    "format_dot",
    "normalize_graph",
    "parse_dot",
    "ValidationError",
    "validate",
    "validate_or_raise",
    "validate_graph",
]
