# Fix Selected-Run Live Cursor and Stream Docs

## Summary
Prevent the app-shell live stream from replaying an entire selected-run journal before the durable journal cursor is known, and update docs/specs to describe the new resource-scoped live stream query shape.

## Implementation Changes
- Gate selected-run live subscription on journal hydration:
  - Keep selected-run detail/status loading in `RunStream` as-is.
  - Add a small shared frontend state flag or selector that indicates whether the selected run’s initial journal load has completed and what its newest sequence is.
  - In `WorkspaceLiveEventsController`, include `run_id` only when the selected run is ready for live tailing:
    - if the selected run has a known `newestSequence`, open with `run_sequence=<newestSequence>`;
    - if the run has an empty loaded journal, open with `run_sequence=0`;
    - if the initial journal is still loading or not yet attempted, omit `run_id` from the live stream.
- Preserve reconnect behavior:
  - Keep cursor values out of the memoized stream identity.
  - Continue storing latest `run_sequence` in refs from incoming live envelopes.
  - On reconnect, include the latest known cursor for the selected run.
  - Changing selected run still creates a new stream once that run’s initial journal cursor is known.
- Adjust tests that currently lock in the wrong behavior:
  - Replace the expectation that the initial selected-run stream lacks `run_sequence`.
  - Assert the stream does not include `run_id` before journal hydration completes.
  - Assert that after initial journal hydration, the stream includes `run_id` and `run_sequence=<latest loaded sequence>`.
  - Assert an empty journal opens live delivery with `run_sequence=0`.
  - Keep the existing “live event advances cursor without reconnect” test.

## Docs and Specs
- Update `README.md`, `specs/spark-ui-ux.md`, `specs/attractor-spec.md`, and `src/spark/guides/spark-operations.md` to describe:
  - `conversation_project_path` as the required scope for `conversation_id`.
  - `runs_project_path` as the optional runs-overview scope, omitted for all-project runs.
  - `triggers_project_path` as the trigger scope.
  - `run_sequence` and `conversation_revision` as reconnect/catch-up cursors, not steady-state stream identity.
- Clarify selected-run UI behavior:
  - selected-run detail and durable journal hydrate first;
  - app-shell live tail starts only after the initial journal cursor is known;
  - live events are additive hints and not the primary full-history source.

## Test Plan
- Run targeted frontend tests:
  - `npm run test:unit -- src/features/runs/__tests__/RunsPanel.test.tsx src/app/__tests__/AppShell.test.tsx`
- Run full verification:
  - `uv run pytest -q`
  - `npm run test:unit`
  - `npm run build`

## Assumptions
- The selected-run journal store is the right place to expose “initial journal loaded” state because it already owns `newestSequence`, status, and journal hydration state.
- A loaded empty journal should use `run_sequence=0` so the backend live endpoint does not need a separate “tail only, no replay” parameter.
