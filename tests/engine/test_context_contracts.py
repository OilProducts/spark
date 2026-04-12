from __future__ import annotations

from attractor.dsl.models import DotAttribute, DotValueType
from attractor.engine.context_contracts import (
    normalize_context_read_key,
    resolve_context_read_contract,
    normalize_context_update_key,
    resolve_context_write_contract,
    validate_context_updates_against_contract,
)


def _string_attr(key: str, value: str) -> DotAttribute:
    return DotAttribute(
        key=key,
        value=value,
        value_type=DotValueType.STRING,
        line=1,
    )


def test_write_contract_parses_and_normalizes_declared_keys() -> None:
    contract = resolve_context_write_contract(
        {
            "spark.writes_context": _string_attr(
                "spark.writes_context",
                '["review.summary","missing_prerequisites"]',
            )
        }
    )

    assert contract.parse_error == ""
    assert contract.allowed_keys == (
        "context.review.summary",
        "missing_prerequisites",
    )


def test_write_contract_reports_malformed_json() -> None:
    contract = resolve_context_write_contract(
        {"spark.writes_context": _string_attr("spark.writes_context", '{"bad":true}')}
    )

    assert contract.allowed_keys == ()
    assert contract.parse_error == "expected a JSON array of strings"


def test_validate_context_updates_against_contract_uses_normalized_keys() -> None:
    contract = resolve_context_write_contract(
        {
            "spark.writes_context": _string_attr(
                "spark.writes_context",
                '["context.review.summary","context.review.required_changes","missing_prerequisites"]'
            )
        }
    )

    violation = validate_context_updates_against_contract(
        {
            "review.summary": "ready",
            "context.review.extra": "nope",
            "missing_prerequisites": [],
        },
        contract,
    )

    assert violation is not None
    assert violation.offending_keys == ("context.review.extra",)
    assert violation.allowed_keys == (
        "context.review.required_changes",
        "context.review.summary",
        "missing_prerequisites",
    )
    assert violation.invalid_keys == ()


def test_write_contract_rejects_invalid_declared_keys() -> None:
    contract = resolve_context_write_contract(
        {"spark.writes_context": _string_attr("spark.writes_context", '[".specflow/state.json"]')}
    )

    assert contract.allowed_keys == ()
    assert contract.parse_error == "invalid context update key '.specflow/state.json': path separators are not allowed"


def test_validate_context_updates_against_contract_rejects_invalid_keys_before_allowlist_check() -> None:
    contract = resolve_context_write_contract(
        {"spark.writes_context": _string_attr("spark.writes_context", '["context.review.summary"]')}
    )

    violation = validate_context_updates_against_contract(
        {
            ".specflow/state.json": "oops",
            "context.review.summary": "ready",
        },
        contract,
    )

    assert violation is not None
    assert violation.invalid_keys == (".specflow/state.json",)
    assert violation.offending_keys == ()


def test_normalize_context_update_key_keeps_bare_keys_and_normalizes_shorthand() -> None:
    assert normalize_context_update_key("missing_prerequisites") == "missing_prerequisites"
    assert normalize_context_update_key("review.summary") == "context.review.summary"
    assert normalize_context_update_key(".specflow/state.json") == ".specflow/state.json"


def test_read_contract_parses_and_normalizes_declared_keys() -> None:
    contract = resolve_context_read_contract(
        {
            "spark.reads_context": _string_attr(
                "spark.reads_context",
                '["review.summary","internal.run_id","missing_prerequisites"]',
            )
        }
    )

    assert contract.parse_error == ""
    assert contract.declared_keys == (
        "context.review.summary",
        "internal.run_id",
        "missing_prerequisites",
    )


def test_read_contract_preserves_explicit_dotted_non_context_keys() -> None:
    contract = resolve_context_read_contract(
        {
            "spark.reads_context": _string_attr(
                "spark.reads_context",
                '["review.summary","custom.live.binding","internal.run_id"]',
            )
        }
    )

    assert contract.parse_error == ""
    assert contract.declared_keys == (
        "context.review.summary",
        "custom.live.binding",
        "internal.run_id",
    )


def test_read_contract_reports_malformed_json() -> None:
    contract = resolve_context_read_contract(
        {"spark.reads_context": _string_attr("spark.reads_context", '{"bad":true}')}
    )

    assert contract.declared_keys == ()
    assert contract.parse_error == "expected a JSON array of strings"


def test_read_contract_rejects_invalid_declared_keys() -> None:
    contract = resolve_context_read_contract(
        {"spark.reads_context": _string_attr("spark.reads_context", '[".specflow/state.json"]')}
    )

    assert contract.declared_keys == ()
    assert contract.parse_error == "invalid context read key '.specflow/state.json': path separators are not allowed"


def test_normalize_context_read_key_preserves_explicit_dotted_non_context_keys() -> None:
    assert normalize_context_read_key("review.summary") == "context.review.summary"
    assert normalize_context_read_key("custom.live.binding") == "custom.live.binding"
    assert normalize_context_read_key(".specflow/state.json") == ".specflow/state.json"
