---
id: CR-2026-0001-unified-llm-conformance-remediation-plan
title: Unified LLM Conformance Remediation Plan
status: completed
type: bugfix
changelog: public
---

## Summary

Delivered the conformance remediation requested for OpenAI tool request payloads, unified content/message validation, provider response normalization under stricter constructors, and Anthropic structured-output support. Known `ContentKind` values now enforce the tagged payload invariants and role constraints described in the request, while custom string content kinds remain extension-friendly.

## Validation

- `uv run pytest -q`: 389 passed, 16 skipped.
- `uv run ruff check .`: passed.
- `uv build`: source distribution and wheel built successfully.

## Shipped Changes

- Updated `src/unified_llm/adapters/openai.py` to emit OpenAI function tools and named tool choices with nested `function` payloads.
- Updated `src/unified_llm/types.py` so `ContentPart` and `Message` reject invalid known-kind payload combinations, thinking redaction mismatches, and invalid role/kind combinations at construction time.
- Updated provider normalizers in `src/unified_llm/provider_utils/anthropic.py`, `src/unified_llm/provider_utils/gemini.py`, `src/unified_llm/provider_utils/openai.py`, and `src/unified_llm/streaming.py` so normalized thinking, tool-call, and tool-result parts satisfy the stricter public model.
- Updated `src/unified_llm/structured.py`, Anthropic normalization behavior, and derived architecture/contract requirement artifacts to remove advertised Anthropic forced-tool structured-output fallback support while continuing to reject explicit unsupported forced-tool strategy requests.
- Updated regression coverage in `tests/test_types.py`, `tests/adapters/test_openai_adapter.py`, `tests/adapters/test_gemini_adapter.py`, and `tests/test_structured.py`.
