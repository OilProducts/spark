# Pipeline Author Workflow

## Goal

Verify that an author can create or modify a spec-valid flow through structured UI controls and keep the result stable across save/reopen.

## Preconditions

1. An active project is selected.
2. A flow is available to edit, or a new flow can be created.
3. The Editor and inspector surfaces are reachable.

## Workflow

1. Open the Editor area for the active project.
2. Create a new flow or open an existing one.
3. Add or modify graph-level settings through structured controls.
4. Add or modify at least one node.
5. Add or modify at least one edge.
6. Review diagnostics and resolve any blocking issues.
7. Save the flow.
8. Reopen the same flow and verify the edited values persist.

## Expected Outcomes

- Required graph, node, and edge editing is possible without leaving the intended authoring UI.
- Validation exposes blocking issues before run launch.
- Save state is visible and failures are surfaced explicitly.
- Reopened flow state matches the saved intent.
