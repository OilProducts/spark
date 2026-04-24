---
id: CR-2026-0004-fix-unified-backend-review-findings
title: Fix Unified Backend Review Findings
status: completed
type: bugfix
changelog: internal
---

## Summary

Delivered the unified backend review fixes across pipeline execution, project chat, workspace flow launch, and frontend API surfaces. The implementation aligns backend contracts, model/provider/reasoning propagation, retry/continue run semantics, and conversation artifact metadata with the approved request.

## Validation

- `uv run pytest -q`: 1673 passed, 26 skipped.

## Shipped Changes

- Added `attractor.api.codergen_contracts` for structured outcome parsing, response-contract violations, write-contract repair prompts, and plain-text fallback behavior.
- Updated unified backend routing and pipeline execution so model, provider, and reasoning-effort selections flow through launch context, node overrides, retries, and fan-in/codergen handler calls.
- Preserved retry and continue API semantics, including retrying in the same run id from the failed checkpoint and creating continued runs with lineage metadata.
- Updated project chat session handling to resume Codex app server threads, handle request-user-input responses, persist artifacts and flow-run requests, and expose model/token metadata through conversation APIs.
- Updated workspace and frontend API surfaces so flow launch payloads remain backend-neutral by default while preserving explicit provider and reasoning settings.
- Added regression coverage for backend invariance, pipeline retry/continue behavior, project chat behavior, workspace flow payloads, and handler backend calls.
