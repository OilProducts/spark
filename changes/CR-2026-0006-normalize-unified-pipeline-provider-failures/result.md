---
id: CR-2026-0006-normalize-unified-pipeline-provider-failures
title: Normalize Unified Pipeline Provider Failures
status: completed
type: bugfix
changelog: internal
---

## Summary
Unified provider backend failures are now normalized into runtime `Outcome` failures instead of escaping the pipeline executor when a provider/session raises a regular `Exception`. This preserves the existing timeout and `RuntimeError` behavior and does not catch `BaseException` classes such as cancellation, `KeyboardInterrupt`, or `SystemExit`.

## Validation
- `uv run pytest -q tests/api/test_backend_invariance.py`
- `uv run pytest -q`
- Latest full-suite result: 1679 passed, 26 skipped.

## Shipped Changes
- `src/attractor/api/codex_backends.py`: added the generic `Exception` normalization path for `UnifiedAgentBackend.run()` using runtime failure outcomes.
- `tests/api/test_backend_invariance.py`: added backend and pipeline regressions covering provider exceptions, cleanup, runtime failure metadata, and codergen stage artifact/status output for unified provider failures.
