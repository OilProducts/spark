# Reviewer Auditor Workflow

## Goal

Verify that a reviewer can inspect a run after execution and reconstruct what happened from the UI alone.

## Preconditions

1. At least one completed or failed run exists in project history.
2. The Runs area can load run details and related artifacts.

## Workflow

1. Open the Runs area for the active project.
2. Select a specific run from history.
3. Review the summary information for status, timing, model, and project/run context.
4. Inspect the event timeline.
5. Inspect checkpoint and context views when available.
6. Open available artifacts and graph render output.
7. Review any visible failure information or routing evidence.

## Expected Outcomes

- Run history is discoverable and stable.
- A reviewer can open a specific run deterministically.
- Checkpoint/context/event/artifact surfaces provide sufficient evidence to understand the run.
- The UI makes failures and execution decisions inspectable without CLI-only dependency.
