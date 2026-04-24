# Normalize Unified Pipeline Provider Failures

## Summary
Close the remaining integration gap by making unified pipeline provider failures return a normal runtime `Outcome`, so codergen stage artifacts and status files are written consistently instead of letting non-`RuntimeError` exceptions escape to the executor.

## Key Changes
- Update `UnifiedAgentBackend.run()` in `src/attractor/api/codex_backends.py`.
  - Keep the existing timeout handling unchanged.
  - Keep `RuntimeError` handling unchanged.
  - Add `except Exception as exc` after the specific handlers and return `self._runtime_failure(str(exc) or exc.__class__.__name__)`.
  - Do not catch `BaseException`, `KeyboardInterrupt`, `SystemExit`, or cancellation outside the existing timeout path.

- Preserve existing behavior outside unified providers.
  - Codex app-server routing remains unchanged.
  - `ProviderRouterBackend` continues dispatching `codex` to Codex and `openai` / `anthropic` / `gemini` to unified.
  - No public API or response schema changes.

## Test Plan
- Add a focused backend regression test in `tests/api/test_backend_invariance.py`.
  - Fake `Session.process_input()` raises `ValueError("provider exploded")`.
  - Assert `UnifiedAgentBackend.run()` returns an `Outcome` with `status=FAIL`, `failure_kind=RUNTIME`, and `failure_reason="provider exploded"`.
  - Assert session/client cleanup still happens.

- Add or extend a handler-level regression if existing fixtures make it cheap.
  - Route a codergen node through unified provider with a fake backend failure.
  - Assert the node fails and stage `status.json` / `response.md` are written with the provider error.

- Run validation:
  - `uv run pytest -q tests/api/test_backend_invariance.py`
  - `uv run pytest -q`
  - Frontend tests are not required for this backend-only patch.

## Assumptions
- Provider SDK failures should be represented as runtime failures, not business or contract failures.
- Cancellation/system-exit style exceptions should remain exceptional and not be normalized.
- This is a narrow follow-up patch; no retry policy or provider-specific error classification is added here.
