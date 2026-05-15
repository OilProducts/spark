# Fix Run Recovery Result IDs

## Summary
Update `run_recovery` so `result_run_id` is only populated when a recovery operation actually starts a run. Retry keeps `result_run_id == source_run_id` on successful start because retry resumes the same run. Continue only gets `result_run_id` when Attractor returns the newly created run id. Failed/validation-error continue attempts leave it empty.

## Implementation Changes
- Adjust run recovery artifact creation/update:
  - For `retry`, keep pending/success behavior as-is: source and result are the same run once retry starts.
  - For `continue`, create the artifact with `result_run_id=""`.
  - On continue success, set `result_run_id` to the returned new run id.
  - On continue validation/error before a new run starts, keep `result_run_id=""` and set `status="failed"` plus `recovery_error`.
- Remove the fallback that writes the source run id into `result_run_id` for failed continue attempts.
- Keep CLI exit behavior unchanged: HTTP/API success still exits `0`; recovery outcome remains expressed in the JSON response via `ok`, `status`, and `error`.

## Test Plan
- Update the existing continue validation-error workspace API test to assert the saved `run_recovery.result_run_id` is empty.
- Add or adjust a successful continue test to assert `result_run_id` is the newly returned run id.
- Keep retry coverage asserting `result_run_id` equals the source run id when retry starts.
- Run:
  - `uv run pytest -q -x --maxfail=1 tests/api/test_project_chat.py tests/test_cli.py`
  - `uv run pytest -q`

## Assumptions
- Empty string is the preferred representation for “no result run exists yet” because the current model uses a string field.
- No behavior change for `observe_run` in this fix; flow-event triggering for recovered runs remains a separate product decision.
