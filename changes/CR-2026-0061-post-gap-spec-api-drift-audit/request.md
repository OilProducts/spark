# Post-Gap Spec/API Drift Audit

## Summary
Audit the Rust-backed Spark implementation against the original Attractor spec/API and current Spark product specs after the known migration gaps have been handled. Classify any remaining runtime, editor, or API drift as fixed, explicitly intentional, a documented non-goal, or a future follow-up. This closes the `post_gap_spec_api_drift_audit` policy gap only when the audit has durable evidence.

## Background
The Rust rewrite migration records now show the earlier known drift candidates as implemented or explicitly resolved. The remaining open policy gap is:

- `post_gap_spec_api_drift_audit`

Its current boundary says that a post-gap audit against the original Attractor spec/API remains pending after known drift candidates are handled. The audit should be final verification, not another broad feature implementation pass.

Primary source material:

- `specs/attractor-spec.md`
- `specs/spark-workspace.md`
- `specs/spark-flow-extensions.md`
- `specs/spark-ui-ux.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`
- Existing compatibility fixtures and validation artifacts under `.spark/rust-rewrite/current/`
- Existing contract, API, frontend, and acceptance workflow tests

## Required Behavior
- Produce a durable post-gap audit artifact that identifies the audited surfaces, evidence used, remaining drift findings, and final classification for each finding.
- Compare the Rust-backed implementation against the original Attractor/Spark contracts for at least:
  - DOT parsing, validation, canonical model conversion, preview, transform, and serialization behavior
  - flow runtime execution semantics, checkpoints, results, events, human gates, manager loop behavior, tool/codergen tasks, and parallel/merge behavior
  - HTTP API route contracts, response envelopes, status codes, deprecated compatibility routes, and CLI/API integration boundaries
  - frontend/editor contracts for structured editing, raw DOT editing, diagnostics, workspace flows, and live run/event surfaces
  - migration boundary records, requirement/decision coverage, and final validation artifacts
- For each drift candidate found, take one of these actions:
  - fix it with executable tests when it is an unintended compatibility regression
  - document it as an intentional Spark product-layer extension with evidence
  - record it as a non-goal or future follow-up only when it is outside the Rust rewrite parity boundary
  - leave it open only if it is genuinely blocked, with a concrete blocker and next action
- Update `.spark/rust-rewrite/current/migration-records.json` so `post_gap_spec_api_drift_audit` is no longer an open policy gap only after the audit evidence exists.
- Update `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json` or regenerate the corresponding validation artifact so coverage no longer reports this item as an unresolved failing policy gap.
- Update `TODO.md` so the final audit item reflects the completed outcome.
- Write `changes/CR-2026-0061-post-gap-spec-api-drift-audit/result.md` summarizing the audit, changes made, classifications, and validation commands.

## Non-Goals
- Do not reopen already approved policy decisions unless the audit finds concrete contradictory evidence.
- Do not remove deprecated compatibility routes just because they are deprecated.
- Do not make broad UI, runtime, or API redesigns while performing the audit.
- Do not weaken tests or migration hygiene checks to make the audit pass.
- Do not mark the policy gap closed based only on source inspection; closure needs durable evidence and validation.

## Suggested Target Paths
- `TODO.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`
- `.spark/rust-rewrite/current/validation/final-validation-artifacts.json`
- `.spark/rust-rewrite/current/validation/post-gap-spec-api-drift-audit.md`
- `changes/CR-2026-0061-post-gap-spec-api-drift-audit/result.md`
- `tests/repo_hygiene/test_rust_rewrite_migration_records.py`
- `tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- Existing focused contract/API/frontend/runtime tests relevant to any drift discovered

## Tests
- Run focused hygiene validation:
  - `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- Run focused compatibility or contract tests for any surface where the audit discovers or fixes drift.
- Run acceptance workflow harness validation if the audit relies on workflow parity evidence.
- Final verification: `uv run pytest -q`.

## Acceptance Criteria
- A durable post-gap audit artifact exists and names the surfaces, source materials, findings, classifications, and evidence.
- Any remaining drift is either fixed with tests, documented as intentional, recorded as a non-goal/future follow-up with rationale, or explicitly blocked with a next action.
- `post_gap_spec_api_drift_audit` is removed from open policy gaps and no longer counts as a failing coverage item.
- `TODO.md` no longer lists the post-gap audit as incomplete.
- `changes/CR-2026-0061-post-gap-spec-api-drift-audit/result.md` records the completed work and validation.
- `uv run pytest -q` passes.
