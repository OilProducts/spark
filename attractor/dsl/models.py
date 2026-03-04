from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, List, Optional, Tuple, Union


class DotValueType(str, Enum):
    STRING = "string"
    INTEGER = "integer"
    FLOAT = "float"
    BOOLEAN = "boolean"
    DURATION = "duration"


@dataclass(frozen=True)
class Duration:
    raw: str
    value: int
    unit: str


DotValue = Union[str, int, float, bool, Duration]


@dataclass
class DotAttribute:
    key: str
    value: DotValue
    value_type: DotValueType
    line: int


@dataclass
class DotNode:
    node_id: str
    attrs: Dict[str, DotAttribute] = field(default_factory=dict)
    line: int = 0
    explicit_attr_keys: set[str] = field(default_factory=set)


@dataclass
class DotEdge:
    source: str
    target: str
    attrs: Dict[str, DotAttribute] = field(default_factory=dict)
    line: int = 0


@dataclass
class DotScopeDefaults:
    node: Dict[str, DotAttribute] = field(default_factory=dict)
    edge: Dict[str, DotAttribute] = field(default_factory=dict)


@dataclass
class DotSubgraphScope:
    id: Optional[str]
    attrs: Dict[str, DotAttribute] = field(default_factory=dict)
    node_ids: List[str] = field(default_factory=list)
    defaults: DotScopeDefaults = field(default_factory=DotScopeDefaults)
    subgraphs: List["DotSubgraphScope"] = field(default_factory=list)


@dataclass
class DotGraph:
    graph_id: str
    graph_attrs: Dict[str, DotAttribute] = field(default_factory=dict)
    nodes: Dict[str, DotNode] = field(default_factory=dict)
    edges: List[DotEdge] = field(default_factory=list)
    defaults: DotScopeDefaults = field(default_factory=DotScopeDefaults)
    subgraphs: List[DotSubgraphScope] = field(default_factory=list)

    @property
    def goal(self) -> str:
        goal_attr = self.graph_attrs.get("goal")
        if goal_attr is None:
            return ""
        return str(goal_attr.value)


class DiagnosticSeverity(str, Enum):
    ERROR = "error"
    WARNING = "warning"
    INFO = "info"


@dataclass
class Diagnostic:
    rule_id: str
    severity: DiagnosticSeverity
    message: str
    line: int = 0
    node_id: Optional[str] = None
    edge: Optional[Tuple[str, str]] = None
    fix: Optional[str] = None

    @property
    def rule(self) -> str:
        return self.rule_id

    @rule.setter
    def rule(self, value: str) -> None:
        self.rule_id = value

    @property
    def node(self) -> Optional[str]:
        return self.node_id

    @node.setter
    def node(self, value: Optional[str]) -> None:
        self.node_id = value


DURATION_UNITS = {"ms", "s", "m", "h", "d"}


def parse_typed_value(token_value: str, token_kind: str) -> tuple[DotValue, DotValueType]:
    """Convert token text into typed DSL value."""
    if token_kind == "STRING":
        return token_value, DotValueType.STRING

    if token_kind == "IDENT":
        lowered = token_value.lower()
        if lowered == "true":
            return True, DotValueType.BOOLEAN
        if lowered == "false":
            return False, DotValueType.BOOLEAN

    if token_kind == "INT":
        raw = token_value
        # Duration literals are lexed as INT + IDENT(unit).
        return int(raw), DotValueType.INTEGER

    if token_kind == "FLOAT":
        return float(token_value), DotValueType.FLOAT

    raise ValueError(f"Unsupported typed value token: kind={token_kind} value={token_value}")
