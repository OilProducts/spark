# Rationalize Runtime Human Gate Surfaces

## Summary
Make Runs the only runtime human-gate operating surface. A pending gate should be answerable only from the Runs pinned `Pending Questions` panel. Other places may show status, location, or audit history, but must not duplicate approval controls. Remove Execution involvement entirely.

## Key Changes
- Keep Runs summary gate awareness:
  - Show blocked/waiting status, current node, and pending question count.
  - Do not add answer controls to the summary card.
- Keep `RunQuestionsPanel` as the only answer surface:
  - Continue rendering grouped pending gates with child-flow/source provenance.
  - Continue submitting answers through `/attractor/pipelines/{id}/questions/{qid}/answer`.
- Keep Run Journal as audit/history:
  - Continue rendering `InterviewStarted`, `InterviewCompleted`, `InterviewTimeout`, and related correlation rows.
  - Do not make journal rows actionable.
- Keep run graph as location/status only:
  - Preserve node waiting state/highlight behavior.
  - Remove the floating `Human Input Required` answer toolbar from run canvas nodes.
- Remove Execution tab human-gate involvement:
  - Delete the pending human-gate banner and `View run` handoff from `ExecutionControls`.
  - Remove any Execution dependency on global `humanGate` state if it becomes unused there.
- Update `specs/spark-ui-ux.md`:
  - State that runtime human gates are operated only in Runs.
  - Change Execution guidance so it does not mention pending human-gate reminders.
  - Clarify that graph/journal/summary expose gate context without owning answers.

## Test Plan
- Update existing frontend tests only; do not add new test files unless an existing suite has no coverage point.
- Adjust tests that expect `execution-pending-human-gate-banner` or node-toolbar answer controls to assert those controls are absent.
- Keep/adjust Runs tests proving:
  - Pending gates render in `RunQuestionsPanel`.
  - Gate answer submission still works from the pinned panel.
  - Summary still shows pending question count / waiting state.
  - Journal still shows interview events.
- Run:
  - `uv run pytest -q`
  - Frontend unit tests covering Runs/Execution if they are not already invoked by pytest in this repo setup.

## Assumptions
- Runtime human gates are distinct from project-chat flow-run-request approvals; this plan only changes runtime `wait.human` gate surfaces.
- The backend API and SSE/journal data flow stay unchanged.
- `humanGate` global state may still be useful for graph waiting indicators; remove it only where inspection shows it is no longer needed.
