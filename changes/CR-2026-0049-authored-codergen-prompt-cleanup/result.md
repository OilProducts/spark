---
id: CR-2026-0049-authored-codergen-prompt-cleanup
title: Authored Codergen Prompt Cleanup
status: completed
type: bugfix
changelog: public
---

## Summary
Codergen prompt resolution now uses only authored, non-empty `prompt` values, then authored, non-empty `label` values, and otherwise sends an empty prompt. Generated/default labels and node IDs are no longer used as implicit codergen prompt fallbacks.

## Validation
Ran `uv run pytest -q`: 1919 passed, 26 skipped.

## Shipped Changes
- Added `has_authored_non_empty_attr(...)` in the DOT model layer and used it for codergen prompt checks.
- Updated executor and `CodergenHandler` prompt resolution to ignore generated labels and node IDs while keeping non-codergen prompt behavior unchanged.
- Routed `emit_event` through filtered backend kwargs, added backend `llm_profile` fallback handling, and updated context read-contract parse error wording.
- Aligned fan-in backend profile fallback behavior with backend `llm_profile`.
- Added and updated tests for authored prompt precedence, authored label fallback, empty prompts without authored text, generated-label exclusion, backend compatibility without `emit_event`, backend profile fallback, validator warnings, and read-contract error wording.
