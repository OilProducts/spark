# Operator Run Workflow

## Goal

Verify that an operator can launch, monitor, and control a run from the UI in active project context.

## Preconditions

1. An active project is selected.
2. A runnable flow is selected.
3. Validation is in a launchable state.

## Workflow

1. Open the Execution area.
2. Verify the selected project and flow context are present.
3. Start a run.
4. Observe visible runtime status changes and stream output.
5. If a run is active long enough, issue a cancel request and confirm the UI reflects the transition.
6. If the run completes, verify the terminal state remains visible.

## Expected Outcomes

- Run start is only available in valid active-project context.
- Runtime state changes are visible while the run is in progress.
- Cancel is explicit and visibly transitions through request/completion states when used.
- Errors or launch failures are surfaced with actionable messaging.
