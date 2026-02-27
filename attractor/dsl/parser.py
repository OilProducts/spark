from __future__ import annotations

import copy
from dataclasses import dataclass, field
import re
from typing import Dict, List, Optional

from .models import (
    DURATION_UNITS,
    DotAttribute,
    DotEdge,
    DotGraph,
    DotNode,
    DotValueType,
    Duration,
    parse_typed_value,
)


NODE_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
ATTR_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$")


class DotParseError(ValueError):
    def __init__(self, message: str, line: int):
        super().__init__(f"line {line}: {message}")
        self.line = line


@dataclass
class Token:
    kind: str
    value: str
    line: int


@dataclass
class _Scope:
    node_defaults: Dict[str, DotAttribute] = field(default_factory=dict)
    edge_defaults: Dict[str, DotAttribute] = field(default_factory=dict)

    def child(self) -> "_Scope":
        return _Scope(
            node_defaults=dict(self.node_defaults),
            edge_defaults=dict(self.edge_defaults),
        )


@dataclass
class _SubgraphState:
    node_ids: set[str] = field(default_factory=set)
    label_value: Optional[object] = None
    label_line: int = 0


def parse_dot(source: str) -> DotGraph:
    tokens = _tokenize(_strip_comments(source))
    parser = _Parser(tokens)
    graph = parser.parse_graph()
    while parser.accept("SEMI"):
        pass
    trailing = parser.current()
    trailing_lower = trailing.value.lower() if trailing.kind == "IDENT" else ""
    if trailing_lower in {"digraph", "graph", "strict"}:
        raise DotParseError("multiple graph declarations are not supported", trailing.line)
    parser.expect("EOF")
    return graph


def normalize_graph(graph: DotGraph) -> DotGraph:
    """Return a deep-copied graph normalized for semantic comparisons."""
    normalized = copy.deepcopy(graph)

    for attr in normalized.graph_attrs.values():
        attr.line = 0

    for node in normalized.nodes.values():
        node.line = 0
        for attr in node.attrs.values():
            attr.line = 0

    for edge in normalized.edges:
        edge.line = 0
        for attr in edge.attrs.values():
            attr.line = 0

    return normalized


class _Parser:
    def __init__(self, tokens: List[Token]):
        self.tokens = tokens
        self.pos = 0

    def current(self) -> Token:
        return self.tokens[self.pos]

    def peek(self, n: int = 1) -> Token:
        return self.tokens[self.pos + n]

    def advance(self) -> Token:
        tok = self.current()
        self.pos += 1
        return tok

    def accept(self, kind: str, value: Optional[str] = None) -> Optional[Token]:
        tok = self.current()
        if tok.kind != kind:
            return None
        if value is not None and tok.value != value:
            return None
        self.pos += 1
        return tok

    def expect(self, kind: str, value: Optional[str] = None) -> Token:
        tok = self.current()
        if tok.kind != kind:
            raise DotParseError(f"expected {kind}, got {tok.kind}", tok.line)
        if value is not None and tok.value != value:
            raise DotParseError(f"expected {value}, got {tok.value}", tok.line)
        self.pos += 1
        return tok

    def parse_graph(self) -> DotGraph:
        first = self.current()
        first_lower = first.value.lower() if first.kind == "IDENT" else ""
        if first_lower == "strict":
            raise DotParseError("strict modifier is not supported", first.line)
        if first_lower == "graph":
            raise DotParseError("undirected graph declarations are not supported", first.line)
        if first.kind == "IDENT" and first_lower == "digraph":
            self.advance()
        else:
            self.expect("IDENT", "digraph")
        graph_id_tok = self.expect("IDENT")
        graph = DotGraph(graph_id=graph_id_tok.value)
        self.expect("LBRACE")

        scope = _Scope()
        while True:
            while self.accept("SEMI"):
                pass
            if self.accept("RBRACE"):
                break
            self.parse_statement(graph, scope, in_subgraph=False, subgraph_state=None)
            while self.accept("SEMI"):
                pass

        return graph

    def parse_statement(
        self,
        graph: DotGraph,
        scope: _Scope,
        *,
        in_subgraph: bool,
        subgraph_state: Optional[_SubgraphState],
    ) -> None:
        tok = self.current()

        if tok.kind == "IDENT" and tok.value == "subgraph":
            self.advance()
            # Optional subgraph id.
            if self.current().kind == "IDENT":
                self.advance()
            self.expect("LBRACE")
            child_scope = scope.child()
            child_subgraph = _SubgraphState()
            while True:
                while self.accept("SEMI"):
                    pass
                if self.accept("RBRACE"):
                    break
                self.parse_statement(graph, child_scope, in_subgraph=True, subgraph_state=child_subgraph)
                while self.accept("SEMI"):
                    pass

            derived_class = _derive_subgraph_class(child_subgraph.label_value)
            if derived_class:
                for node_id in child_subgraph.node_ids:
                    node = graph.nodes.get(node_id)
                    if node:
                        _append_class(node, derived_class, child_subgraph.label_line)

            if subgraph_state is not None:
                subgraph_state.node_ids.update(child_subgraph.node_ids)
            return

        if tok.kind == "IDENT" and tok.value == "graph" and self.peek().kind == "LBRACKET":
            self.advance()
            attrs = self.parse_attr_block()
            if not in_subgraph:
                graph.graph_attrs.update(attrs)
            elif subgraph_state is not None and "label" in attrs:
                label_attr = attrs["label"]
                subgraph_state.label_value = label_attr.value
                subgraph_state.label_line = label_attr.line
            return

        if tok.kind == "IDENT" and tok.value == "node" and self.peek().kind == "LBRACKET":
            self.advance()
            scope.node_defaults.update(self.parse_attr_block())
            return

        if tok.kind == "IDENT" and tok.value == "edge" and self.peek().kind == "LBRACKET":
            self.advance()
            scope.edge_defaults.update(self.parse_attr_block())
            return

        if tok.kind == "IDENT" and self.peek().kind == "EQ":
            key_tok = self.advance()
            if not ATTR_KEY_RE.match(key_tok.value):
                raise DotParseError(f"invalid attribute key '{key_tok.value}'", key_tok.line)
            self.expect("EQ")
            value, value_type, line = self.parse_value()
            if not in_subgraph:
                graph.graph_attrs[key_tok.value] = DotAttribute(
                    key=key_tok.value,
                    value=value,
                    value_type=value_type,
                    line=line,
                )
            elif subgraph_state is not None and key_tok.value == "label":
                subgraph_state.label_value = value
                subgraph_state.label_line = line
            return

        if tok.kind == "IDENT":
            self.parse_node_or_edge(graph, scope, subgraph_state=subgraph_state)
            return

        raise DotParseError(f"unexpected token {tok.kind}:{tok.value}", tok.line)

    def parse_node_or_edge(
        self,
        graph: DotGraph,
        scope: _Scope,
        *,
        subgraph_state: Optional[_SubgraphState],
    ) -> None:
        first = self.expect("IDENT")
        self._validate_node_id(first)

        if self.accept("ARROW"):
            chain_ids = [first]
            next_tok = self.expect("IDENT")
            self._validate_node_id(next_tok)
            chain_ids.append(next_tok)
            while self.accept("ARROW"):
                next_tok = self.expect("IDENT")
                self._validate_node_id(next_tok)
                chain_ids.append(next_tok)

            stmt_attrs = self.parse_attr_block() if self.current().kind == "LBRACKET" else {}
            effective = dict(scope.edge_defaults)
            effective.update(stmt_attrs)

            for idx in range(len(chain_ids) - 1):
                src = chain_ids[idx]
                dst = chain_ids[idx + 1]
                graph.edges.append(
                    DotEdge(
                        source=src.value,
                        target=dst.value,
                        attrs=_clone_attrs(effective),
                        line=src.line,
                    )
                )
            return

        stmt_attrs = self.parse_attr_block() if self.current().kind == "LBRACKET" else {}
        effective = _clone_attrs(scope.node_defaults)
        effective.update(stmt_attrs)

        existing = graph.nodes.get(first.value)
        if existing:
            merged = dict(existing.attrs)
            for key, attr in scope.node_defaults.items():
                if key not in merged:
                    merged[key] = DotAttribute(
                        key=attr.key,
                        value=copy.deepcopy(attr.value),
                        value_type=attr.value_type,
                        line=attr.line,
                    )
            merged.update(stmt_attrs)
            existing.attrs = merged
            existing.explicit_attr_keys.update(stmt_attrs.keys())
            if subgraph_state is not None:
                subgraph_state.node_ids.add(first.value)
            return

        graph.nodes[first.value] = DotNode(
            node_id=first.value,
            attrs=effective,
            line=first.line,
            explicit_attr_keys=set(stmt_attrs.keys()),
        )
        if subgraph_state is not None:
            subgraph_state.node_ids.add(first.value)

    def parse_attr_block(self) -> Dict[str, DotAttribute]:
        self.expect("LBRACKET")
        attrs: Dict[str, DotAttribute] = {}

        if self.accept("RBRACKET"):
            return attrs

        while True:
            key_tok = self.expect("IDENT")
            if not ATTR_KEY_RE.match(key_tok.value):
                raise DotParseError(f"invalid attribute key '{key_tok.value}'", key_tok.line)
            self.expect("EQ")
            value, value_type, value_line = self.parse_value()
            if key_tok.value == "class" and value_type == DotValueType.STRING:
                value = _normalize_class_list(str(value))
            attrs[key_tok.value] = DotAttribute(
                key=key_tok.value,
                value=value,
                value_type=value_type,
                line=value_line,
            )

            if self.accept("RBRACKET"):
                break
            if not self.accept("COMMA"):
                tok = self.current()
                raise DotParseError("commas are required between attributes", tok.line)
            if self.current().kind == "RBRACKET":
                raise DotParseError("trailing comma is not allowed in attribute blocks", self.current().line)

        return attrs

    def parse_value(self) -> tuple[object, DotValueType, int]:
        tok = self.current()

        if tok.kind == "INT" and self.peek().kind == "IDENT" and self.peek().value in DURATION_UNITS:
            int_tok = self.advance()
            unit_tok = self.advance()
            numeric_value = int(int_tok.value)
            raw = f"{numeric_value}{unit_tok.value}"
            return Duration(raw=raw, value=numeric_value, unit=unit_tok.value), DotValueType.DURATION, int_tok.line

        if tok.kind in {"STRING", "INT", "FLOAT", "IDENT"}:
            val_tok = self.advance()
            if val_tok.kind == "IDENT":
                if val_tok.value in {"true", "false"}:
                    value, value_type = parse_typed_value(val_tok.value, val_tok.kind)
                    return value, value_type, val_tok.line
                # Bare identifiers are accepted as string-like enums (e.g. rankdir=LR).
                return val_tok.value, DotValueType.STRING, val_tok.line

            value, value_type = parse_typed_value(val_tok.value, val_tok.kind)
            return value, value_type, val_tok.line

        raise DotParseError(f"invalid value token {tok.kind}:{tok.value}", tok.line)

    def _validate_node_id(self, token: Token) -> None:
        if not NODE_ID_RE.match(token.value):
            raise DotParseError(
                f"invalid node id '{token.value}', must match [A-Za-z_][A-Za-z0-9_]*",
                token.line,
            )


def _tokenize(source: str) -> List[Token]:
    tokens: List[Token] = []
    i = 0
    line = 1
    n = len(source)

    while i < n:
        ch = source[i]

        # Whitespace
        if ch in " \t\r":
            i += 1
            continue
        if ch == "\n":
            line += 1
            i += 1
            continue

        # Line comments: // ...
        if ch == "/" and i + 1 < n and source[i + 1] == "/":
            i += 2
            while i < n and source[i] != "\n":
                i += 1
            continue

        # Block comments: /* ... */
        if ch == "/" and i + 1 < n and source[i + 1] == "*":
            i += 2
            while i + 1 < n and not (source[i] == "*" and source[i + 1] == "/"):
                if source[i] == "\n":
                    line += 1
                i += 1
            if i + 1 >= n:
                raise DotParseError("unterminated block comment", line)
            i += 2
            continue

        # Operators / punctuation
        if ch == "-" and i + 1 < n and source[i + 1] == ">":
            tokens.append(Token("ARROW", "->", line))
            i += 2
            continue

        if ch == "-" and i + 1 < n and source[i + 1] == "-":
            raise DotParseError("undirected edges ('--') are not supported", line)

        if (
            ch == "-"
            and i + 1 < n
            and (source[i + 1].isalpha() or source[i + 1] == "_")
            and tokens
            and tokens[-1].kind == "IDENT"
            and (len(tokens) == 1 or tokens[-2].kind in {"LBRACE", "SEMI", "ARROW", "RBRACE"})
        ):
            j = i + 1
            while j < n and (source[j].isalnum() or source[j] in "_-"):
                j += 1
            k = j
            while k < n and source[k] in " \t\r":
                k += 1
            if k >= n or source[k] != "=":
                invalid_id = f"{tokens[-1].value}-{source[i + 1:j]}"
                raise DotParseError(
                    f"invalid node id '{invalid_id}', must match [A-Za-z_][A-Za-z0-9_]*",
                    line,
                )

        punct = {
            "{": "LBRACE",
            "}": "RBRACE",
            "[": "LBRACKET",
            "]": "RBRACKET",
            ",": "COMMA",
            ";": "SEMI",
            "=": "EQ",
        }
        if ch in punct:
            tokens.append(Token(punct[ch], ch, line))
            i += 1
            continue

        # String literal
        if ch == '"':
            start_line = line
            i += 1
            value_chars: List[str] = []
            while i < n:
                c = source[i]
                if c == "\\":
                    if i + 1 >= n:
                        raise DotParseError("unterminated escape sequence", start_line)
                    esc = source[i + 1]
                    mapping = {
                        '"': '"',
                        "n": "\n",
                        "t": "\t",
                        "\\": "\\",
                    }
                    if esc not in mapping:
                        raise DotParseError(f"unsupported escape \\{esc}", line)
                    value_chars.append(mapping[esc])
                    i += 2
                    continue
                if c == '"':
                    i += 1
                    break
                if c == "\n":
                    raise DotParseError("unescaped newline in string literal", line)
                value_chars.append(c)
                i += 1
            else:
                raise DotParseError("unterminated string literal", start_line)

            tokens.append(Token("STRING", "".join(value_chars), start_line))
            continue

        # Number: int/float with optional leading sign.
        starts_number = ch.isdigit()
        starts_signed_number = (
            ch in "+-"
            and i + 1 < n
            and (
                source[i + 1].isdigit()
                or (source[i + 1] == "." and i + 2 < n and source[i + 2].isdigit())
            )
        )
        starts_leading_dot_float = ch == "." and i + 1 < n and source[i + 1].isdigit()

        if starts_number or starts_signed_number or starts_leading_dot_float:
            start = i
            start_line = line
            if source[i] in "+-":
                i += 1
            while i < n and source[i].isdigit():
                i += 1

            if i < n and source[i] == ".":
                i += 1
                if i >= n or not source[i].isdigit():
                    raise DotParseError("invalid float literal", start_line)
                while i < n and source[i].isdigit():
                    i += 1
                tokens.append(Token("FLOAT", source[start:i], start_line))
            else:
                tokens.append(Token("INT", source[start:i], start_line))
            continue

        # Identifier / qualified identifier
        if ch.isalpha() or ch == "_":
            start = i
            i += 1
            while i < n and (source[i].isalnum() or source[i] in "_."):
                i += 1
            tokens.append(Token("IDENT", source[start:i], line))
            continue

        if ch == "<":
            raise DotParseError("HTML-like labels are not supported", line)

        if ch == ":":
            raise DotParseError("port and compass point syntax is not supported", line)

        raise DotParseError(f"unexpected character '{ch}'", line)

    tokens.append(Token("EOF", "", line))
    return tokens


def _strip_comments(source: str) -> str:
    out: List[str] = []
    i = 0
    line = 1
    n = len(source)
    in_string = False

    while i < n:
        ch = source[i]

        if in_string:
            out.append(ch)
            if ch == "\\":
                if i + 1 >= n:
                    raise DotParseError("unterminated escape sequence", line)
                out.append(source[i + 1])
                i += 2
                continue
            if ch == '"':
                in_string = False
            if ch == "\n":
                line += 1
            i += 1
            continue

        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue

        if ch == "/" and i + 1 < n and source[i + 1] == "/":
            i += 2
            while i < n and source[i] != "\n":
                i += 1
            continue

        if ch == "/" and i + 1 < n and source[i + 1] == "*":
            i += 2
            while i + 1 < n and not (source[i] == "*" and source[i + 1] == "/"):
                if source[i] == "\n":
                    line += 1
                    out.append("\n")
                i += 1
            if i + 1 >= n:
                raise DotParseError("unterminated block comment", line)
            i += 2
            continue

        if ch == "\n":
            line += 1
        out.append(ch)
        i += 1

    return "".join(out)


def _clone_attrs(attrs: Dict[str, DotAttribute]) -> Dict[str, DotAttribute]:
    return {
        key: DotAttribute(
            key=attr.key,
            value=copy.deepcopy(attr.value),
            value_type=attr.value_type,
            line=attr.line,
        )
        for key, attr in attrs.items()
    }


def _derive_subgraph_class(label_value: Optional[object]) -> str:
    if label_value is None:
        return ""
    normalized = re.sub(r"\s+", "-", str(label_value).strip().lower())
    normalized = re.sub(r"[^a-z0-9-]", "", normalized)
    normalized = re.sub(r"-{2,}", "-", normalized)
    return normalized.strip("-")


def _append_class(node: DotNode, class_name: str, line: int) -> None:
    existing_attr = node.attrs.get("class")
    if existing_attr is None:
        node.attrs["class"] = DotAttribute(
            key="class",
            value=class_name,
            value_type=DotValueType.STRING,
            line=line or node.line,
        )
        return

    classes = [c.strip() for c in str(existing_attr.value).split(",") if c.strip()]
    if class_name in classes:
        return
    existing_attr.value = ",".join(classes + [class_name])
    existing_attr.value_type = DotValueType.STRING


def _normalize_class_list(raw: str) -> str:
    seen: set[str] = set()
    ordered: list[str] = []
    for class_name in [c.strip().lower() for c in raw.split(",")]:
        if class_name == "" or class_name in seen:
            continue
        seen.add(class_name)
        ordered.append(class_name)
    return ",".join(ordered)
