---
id: CR-2026-0043-fix-run-recovery-result-ids
title: Fix Run Recovery Result IDs
status: completed
type: bugfix
changelog: internal
---

## Summary
Run recovery records now distinguish between retrying the same run and continuing into a newly created run. Retry keeps the source run id as the result run id when it starts, while continue recoveries begin with an empty result run id and only record a result after Attractor returns a new run id.

## Validation
- `uv run pytest -q -x --maxfail=1 tests/api/test_project_chat.py tests/test_cli.py` passed with 173 tests.
- `uv run pytest -q` passed with 1991 tests, 26 skipped, and 2 deprecation warnings from websocket dependencies.

## Shipped Changes
- Updated workspace run recovery API handling so failed continue attempts and continue validation errors leave `result_run_id` empty and mark the recovery failed with `recovery_error`.
- Updated run recovery artifact creation so omitted result ids default to the source run only for retry operations, and default to empty for continue operations.
- Extended workspace API coverage for continue validation errors to assert the saved `run_recovery.result_run_id` remains empty; existing retry and successful continue coverage verifies retry uses the source run id and continue records the returned new run id.
