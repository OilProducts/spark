---
id: CR-2026-0058-restore-proposedplan-plan-card-compatibility
title: Restore Proposed Plan Plan Card Compatibility
status: completed
type: bugfix
changelog: public
---

## Summary

Plan-mode assistant final text now supports textual `<proposed_plan>...</proposed_plan>` blocks so providers that return proposed-plan markup as plain text still produce the normal plan segment and pending proposed-plan artifact. The structured Codex plan-channel path remains authoritative when present and does not duplicate plan artifacts.

## Validation

- `uv run pytest -q` passed: 1958 passed, 26 skipped.

## Shipped Changes

- Added proposed-plan extraction for plan-mode final assistant text, including case-insensitive tag detection, plan text normalization, and removal of the plan block from ordinary assistant prose.
- Materialized extracted textual plans as completed `plan` segments with pending `proposed_plans` sidecars, while preserving surrounding final-answer prose as an assistant message segment when present.
- Preserved non-plan chat behavior so `<proposed_plan>` markup remains ordinary assistant text outside plan mode.
- Preserved structured plan-channel behavior so existing plan events continue to create exactly one plan card/artifact without duplication from final text.
- Added regression coverage in the project chat API tests for plan-only textual markup, markup with surrounding prose, structured plan-channel precedence, and non-plan chat mode.
