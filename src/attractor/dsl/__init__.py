"""DOT DSL parsing and validation."""

from .models import Diagnostic, DiagnosticSeverity, DotGraph
from .formatter import canonicalize_dot, canonicalize_readable_dot, format_dot, format_readable_dot
from .parser import DotParseError, normalize_graph, parse_dot
from .validator import (
    ValidationError,
    clear_registered_lint_rules,
    register_lint_rule,
    validate,
    validate_graph,
    validate_or_raise,
)

__all__ = [
    "Diagnostic",
    "DiagnosticSeverity",
    "DotGraph",
    "DotParseError",
    "canonicalize_dot",
    "canonicalize_readable_dot",
    "format_dot",
    "format_readable_dot",
    "normalize_graph",
    "parse_dot",
    "ValidationError",
    "register_lint_rule",
    "clear_registered_lint_rules",
    "validate",
    "validate_or_raise",
    "validate_graph",
]
