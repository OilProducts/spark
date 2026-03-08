# Project Select, Author, Execute, Inspect

## Goal

Verify that a user can complete a full project-scoped workflow in the UI without falling back to raw DOT for the required steps in this scenario.

## Preconditions

1. At least one local project directory is available for registration.
2. The selected project is a Git repository and is usable as a working directory.
3. At least one runnable flow exists or can be created during the workflow.
4. The UI can reach the backend APIs needed for project selection, flow editing, execution, and run inspection.

## Workflow

1. Open the Home area.
2. Register or select a project.
3. Confirm the project becomes active in navigation and Home.
4. Open or create a conversation thread for the active project.
5. Review the project-scoped chat surface and confirm the thread is empty or resumes prior history.
6. Switch to the Editor area and open a flow.
7. Edit graph, node, or edge configuration through structured UI controls.
8. Run validation and resolve any blocking diagnostics from the UI.
9. Save the flow and reopen it to confirm the edited values persist.
10. Open the Execution area and launch a run in the active project context.
11. Observe live status, stream output, and any human-gate or failure state messaging.
12. Open the completed run in the Runs area.
13. Inspect summary, events, checkpoint/context, and available artifacts.
14. Return to authoring, make a follow-up edit, and launch another run in the same project.

## Expected Outcomes

- The workflow is completable through the intended UI surfaces without an implementation dead-end.
- Project scope remains consistent across Home, Editor, Execution, and Runs.
- Flow edits persist and rehydrate without silent behavior loss.
- Execution and run inspection provide enough visible information to diagnose outcomes and continue iterating.
