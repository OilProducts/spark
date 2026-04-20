from __future__ import annotations

import json

from attractor.engine.context_contracts import ContextWriteContract


_STATUS_ENVELOPE_SCHEMA_LINES: tuple[str, ...] = (
    "Structured response contract:",
    '- Return ONLY a JSON object.',
    '- Required top-level key: "outcome" with one of "success", "fail", "partial_success", or "retry".',
    '- Optional top-level keys: "preferred_label", "suggested_next_ids", "context_updates", "notes", "failure_reason", and "retryable".',
    '- Use "preferred_label" for routing.',
    '- "suggested_next_ids" must be a list of strings.',
    '- "context_updates" must be a JSON object.',
    '- Do not emit any other top-level keys.',
)


def build_status_envelope_prompt_appendix(write_contract: ContextWriteContract | None) -> str:
    return "\n".join(
        [
            *_STATUS_ENVELOPE_SCHEMA_LINES,
            build_status_envelope_context_updates_contract_text(write_contract),
            '- If no routing or context updates are needed, prefer: {"outcome":"success"}',
        ]
    )


def build_status_envelope_context_updates_contract_text(
    write_contract: ContextWriteContract | None,
) -> str:
    allowed_keys = _allowed_context_update_keys(write_contract)
    lines = ['Node-specific "context_updates" rules:']
    if not allowed_keys:
        lines.append('- This node must not emit "context_updates".')
        return "\n".join(lines)

    lines.extend(
        [
            '- This node may include "context_updates" only when needed.',
            (
                '- Allowed "context_updates" keys for this node, and no others: '
                f"{_format_allowed_keys(allowed_keys)}."
            ),
            '- Inside "context_updates", emit a flat key/value map using the literal keys above.',
            '- Use JSON null to clear a previously set context key when this node needs to end or reset live state.',
        ]
    )

    dotted_key = next((key for key in allowed_keys if "." in key), None)
    if dotted_key:
        lines.extend(
            [
                f'- Keys with dots stay literal keys, for example "{dotted_key}".',
                (
                    '- Do not nest objects inside "context_updates" for dotted keys. '
                    f"Use {_flat_context_updates_example(dotted_key)} not {_nested_context_updates_example(dotted_key)}."
                ),
            ]
        )
    return "\n".join(lines)


def format_status_envelope_allowed_keys(write_contract: ContextWriteContract | None) -> str:
    return _format_allowed_keys(_allowed_context_update_keys(write_contract))


def _allowed_context_update_keys(write_contract: ContextWriteContract | None) -> tuple[str, ...]:
    if write_contract is None:
        return ()
    return tuple(write_contract.allowed_keys)


def _format_allowed_keys(allowed_keys: tuple[str, ...]) -> str:
    if not allowed_keys:
        return "<none>"
    return ", ".join(f'"{key}"' for key in allowed_keys)


def _flat_context_updates_example(key: str) -> str:
    return json.dumps({"context_updates": {key: "..."}}, separators=(",", ":"))


def _nested_context_updates_example(key: str) -> str:
    nested_value: object = "..."
    for segment in reversed(key.split(".")):
        nested_value = {segment: nested_value}
    return json.dumps({"context_updates": nested_value}, separators=(",", ":"))
