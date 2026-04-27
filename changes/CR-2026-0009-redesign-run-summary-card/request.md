# Redesign Run Summary Card

## Summary
Refactor the Runs tab summary card from a broad metadata grid into a focused run-orientation panel. The card should answer: what is happening now, did it work, what work is this, and what did it cost. No backend API changes.

## Key Changes
- Update `specs/spark-ui-ux.md` to clarify that the selected-run summary should prioritize current activity, outcome, scope, and usage over execution-init metadata.
- Restructure `RunSummaryCard` into four visible sections:
  - `Now`: status, current node, latest journal summary, completed node count, pending question count, retry/waiting signal when present.
  - `Outcome`: lifecycle status, workflow outcome, duration, started/ended timestamps, and failure/outcome reason only when present.
  - `Scope`: flow name, project path or active-project fallback, compact git ref, spec/plan artifacts when present, and lineage only when applicable.
  - `Usage`: total tokens, estimated cost, input/cached/output counts, cost note, and always-visible per-model breakdown when model rows exist.
- Remove always-visible low-value rows from the summary:
  - hide working directory unless it differs from project path, in which case show a compact “Working dir differs” note.
  - omit empty `—` rows for last error, lineage, artifacts, git, and usage details.
  - do not show root run, parent run, child invocation, and continued-from as separate rows; combine applicable lineage into one compact row.
- Keep existing run-level actions in the summary header area:
  - cancel for active runs
  - retry for failed runs
  - continue for inactive runs
  - collapse toggle

## Interface And State
- No backend schema or transport changes.
- Keep `RunSummaryCard` props mostly stable, but allow `monitoringFacts` to be replaced or reshaped into summary-specific view-model sections if that makes the component simpler.
- Preserve existing `data-testid` coverage where reasonable, but update tests to assert the new section-level contract rather than the old one-row-per-field grid.
- Keep project path in the main summary using `run.project_path || activeProjectPath || '—'`; do not show working directory in normal cases.

## Test Plan
- Update RunsPanel summary tests to assert:
  - summary renders `Now`, `Outcome`, `Scope`, and `Usage` sections in order.
  - active runs show current node/latest activity/pending questions and cancel action.
  - completed failure runs show status separately from outcome and surface outcome reason or last error.
  - working directory is hidden when it matches or is not meaningfully different from project path, and only appears as a difference note when needed.
  - lineage appears as one compact row only when continued/parent/root/child data exists.
  - live token telemetry updates the compact usage section and always-visible per-model breakdown.
- Run:
  - `npm --prefix frontend run test:unit -- RunsPanel`
  - `npm --prefix frontend run build`
  - `uv run pytest -q`

## Assumptions
- Usage per-model details remain visible when available, but grouped under one Usage section.
- Project path remains the operator-facing scope in the summary; working directory is diagnostic evidence, not normal summary content.
- This is a frontend/spec refactor only; no Attractor or Workspace API changes.
