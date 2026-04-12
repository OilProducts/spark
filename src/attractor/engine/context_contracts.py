from __future__ import annotations

from dataclasses import dataclass
from functools import lru_cache
import json
import re
from typing import Collection, Mapping

from attractor.dsl.models import DotAttribute


_ALLOWED_CONTEXT_UPDATE_PREFIXES: tuple[str, ...] = (
    "context.",
    "graph.",
    "internal.",
    "parallel.",
    "stack.",
    "human.gate.",
    "work.",
    "_attractor.",
)
_CONTEXT_READ_SHORTHAND_PREFIXES: tuple[str, ...] = (
    "request.",
    "review.",
    "planflow.",
    "milestone.",
    "item.",
)
_CONTEXT_UPDATE_KEY_RE = re.compile(r"^[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*$")


@dataclass(frozen=True)
class ContextWriteContract:
    allowed_keys: tuple[str, ...] = ()
    parse_error: str = ""


@dataclass(frozen=True)
class ContextReadContract:
    declared_keys: tuple[str, ...] = ()
    parse_error: str = ""


@dataclass(frozen=True)
class ContextWriteContractViolation:
    offending_keys: tuple[str, ...]
    allowed_keys: tuple[str, ...]
    parse_error: str = ""
    invalid_keys: tuple[str, ...] = ()

    def format_reason(self, *, node_id: str | None = None) -> str:
        node_prefix = f" for node '{node_id}'" if node_id else ""
        offending = ", ".join(self.offending_keys) if self.offending_keys else "<none>"
        declared = ", ".join(self.allowed_keys) if self.allowed_keys else "<none>"
        invalid = ", ".join(self.invalid_keys) if self.invalid_keys else "<none>"
        parts = []
        if self.invalid_keys:
            parts.append(f"invalid context_updates keys{node_prefix}: {invalid}")
            parts.append("context_updates keys must be bare identifiers or dot-separated identifiers")
        parts.extend(
            [
                f"undeclared context_updates keys{node_prefix}: {offending}",
                f"declared spark.writes_context allowlist: {declared}",
            ]
        )
        if self.parse_error:
            parts.append(f"spark.writes_context parse error: {self.parse_error}")
        return "; ".join(parts)


def normalize_context_key(key: str) -> str:
    normalized = str(key).strip()
    if not normalized or "." not in normalized:
        return normalized
    if _invalid_context_key_reason(normalized):
        return normalized
    if any(normalized.startswith(prefix) for prefix in _ALLOWED_CONTEXT_UPDATE_PREFIXES):
        return normalized
    return f"context.{normalized}"


def normalize_context_update_key(key: str) -> str:
    return normalize_context_key(key)


def normalize_context_read_key(key: str) -> str:
    normalized = str(key).strip()
    if not normalized or "." not in normalized:
        return normalized
    if _invalid_context_key_reason(normalized):
        return normalized
    if any(normalized.startswith(prefix) for prefix in _ALLOWED_CONTEXT_UPDATE_PREFIXES):
        return normalized
    if any(normalized.startswith(prefix) for prefix in _CONTEXT_READ_SHORTHAND_PREFIXES):
        return f"context.{normalized}"
    return normalized


def resolve_context_write_contract(node_attrs: Mapping[str, DotAttribute]) -> ContextWriteContract:
    attr = node_attrs.get("spark.writes_context")
    raw_value = None if attr is None else str(attr.value)
    return _parse_context_write_contract(raw_value)


def resolve_context_read_contract(node_attrs: Mapping[str, DotAttribute]) -> ContextReadContract:
    attr = node_attrs.get("spark.reads_context")
    raw_value = None if attr is None else str(attr.value)
    return _parse_context_read_contract(raw_value)


def validate_context_updates_against_contract(
    context_updates: Mapping[str, object],
    contract: ContextWriteContract,
    *,
    exempt_keys: Collection[str] = (),
    exempt_prefixes: Collection[str] = (),
) -> ContextWriteContractViolation | None:
    normalized_authored_keys: set[str] = set()
    invalid_authored_keys: set[str] = set()
    for raw_key in context_updates.keys():
        raw_key_text = str(raw_key).strip()
        normalized_key = normalize_context_update_key(raw_key_text)
        if _is_exempt_context_update_key(
            normalized_key,
            exempt_keys=exempt_keys,
            exempt_prefixes=exempt_prefixes,
        ):
            continue
        invalid_reason = _invalid_context_key_reason(raw_key_text)
        if invalid_reason:
            invalid_authored_keys.add(raw_key_text)
            continue
        normalized_authored_keys.add(normalized_key)
    if not normalized_authored_keys and not invalid_authored_keys:
        return None

    allowed = set(contract.allowed_keys)
    offending = tuple(sorted(key for key in normalized_authored_keys if key not in allowed))
    invalid_keys = tuple(sorted(invalid_authored_keys))
    if not offending and not invalid_keys and not contract.parse_error:
        return None
    if contract.parse_error and not offending and not invalid_keys:
        offending = tuple(sorted(normalized_authored_keys))
    return ContextWriteContractViolation(
        offending_keys=offending,
        allowed_keys=contract.allowed_keys,
        parse_error=contract.parse_error,
        invalid_keys=invalid_keys,
    )


def _is_exempt_context_update_key(
    key: str,
    *,
    exempt_keys: Collection[str],
    exempt_prefixes: Collection[str],
) -> bool:
    if key in exempt_keys:
        return True
    return any(key.startswith(prefix) for prefix in exempt_prefixes)


@lru_cache(maxsize=None)
def _parse_context_write_contract(raw_value: str | None) -> ContextWriteContract:
    parsed_contract = _parse_context_key_contract(
        raw_value,
        normalize_key=normalize_context_update_key,
        empty_key_error="expected non-empty context update keys",
        invalid_key_label="context update key",
    )
    return ContextWriteContract(
        allowed_keys=parsed_contract.keys,
        parse_error=parsed_contract.parse_error,
    )


@lru_cache(maxsize=None)
def _parse_context_read_contract(raw_value: str | None) -> ContextReadContract:
    parsed_contract = _parse_context_key_contract(
        raw_value,
        normalize_key=normalize_context_read_key,
        empty_key_error="expected non-empty context read keys",
        invalid_key_label="context read key",
    )
    return ContextReadContract(
        declared_keys=parsed_contract.keys,
        parse_error=parsed_contract.parse_error,
    )


@dataclass(frozen=True)
class _ParsedContextKeyContract:
    keys: tuple[str, ...] = ()
    parse_error: str = ""


def _parse_context_key_contract(
    raw_value: str | None,
    *,
    normalize_key,
    empty_key_error: str,
    invalid_key_label: str,
) -> _ParsedContextKeyContract:
    normalized_raw = "" if raw_value is None else raw_value.strip()
    if not normalized_raw:
        return _ParsedContextKeyContract()

    try:
        parsed = json.loads(normalized_raw)
    except json.JSONDecodeError as exc:
        return _ParsedContextKeyContract(
            parse_error=f"expected a JSON array of strings: {exc.msg} at line {exc.lineno} column {exc.colno}"
        )

    if not isinstance(parsed, list):
        return _ParsedContextKeyContract(parse_error="expected a JSON array of strings")
    if any(not isinstance(item, str) for item in parsed):
        return _ParsedContextKeyContract(parse_error="expected a JSON array of strings")

    normalized_keys: set[str] = set()
    for item in parsed:
        normalized_item = str(item).strip()
        if not normalized_item:
            return _ParsedContextKeyContract(parse_error=empty_key_error)
        invalid_reason = _invalid_context_key_reason(normalized_item)
        if invalid_reason:
            return _ParsedContextKeyContract(
                parse_error=f"invalid {invalid_key_label} '{normalized_item}': {invalid_reason}"
            )
        normalized_item = normalize_key(normalized_item)
        if not normalized_item:
            return _ParsedContextKeyContract(parse_error=empty_key_error)
        normalized_keys.add(normalized_item)
    return _ParsedContextKeyContract(keys=tuple(sorted(normalized_keys)))


def _invalid_context_key_reason(key: str) -> str:
    normalized = str(key).strip()
    if not normalized:
        return "keys must be non-empty"
    if "/" in normalized or "\\" in normalized:
        return "path separators are not allowed"
    if normalized.startswith(".") or normalized.endswith(".") or ".." in normalized:
        return "empty dotted segments are not allowed"
    if not _CONTEXT_UPDATE_KEY_RE.fullmatch(normalized):
        return "keys must be bare identifiers or dot-separated identifiers"
    return ""
