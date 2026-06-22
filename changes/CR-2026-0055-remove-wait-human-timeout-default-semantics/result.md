---
id: CR-2026-0055-remove-wait-human-timeout-default-semantics
title: Remove wait.human timeout/default semantics
status: completed
type: bugfix
changelog: public
---

## Summary
`wait.human` is now an indefinite human gate that only routes after an explicit answer matching one of its outgoing edge choices. Timeout/default semantics were removed from the interviewer model, runtime broker, container question payloads, event summaries, frontend authoring surfaces, serializers, docs, and tests.

## Validation
- `uv run pytest -q` passed with 1945 tests passed and 26 skipped.

## Shipped Changes
- Removed `Question.default`, `Question.timeout_seconds`, and `AnswerValue.TIMEOUT` from the interviewer model and payload plumbing.
- Simplified human gate runtime behavior: explicit selections route, skipped/empty/unmatched answers fail as `human skipped interaction`, and missing outgoing edges fail without a default route.
- Removed broker timeout/default fallback behavior and stopped emitting or summarizing `InterviewTimeout`.
- Removed `human.default_choice` as a first-class authoring attribute from backend graph payloads, frontend inspector/canvas controls, canonical serialization, contract expectations, and docs.
- Updated wait-human, broker, execution-container, frontend contract, and question model tests to cover the explicit-selection behavior and absence of timeout/default fields.
