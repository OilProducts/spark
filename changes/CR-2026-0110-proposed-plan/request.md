## Spark task board and assistant controls

### Summary

Add a project-scoped **Tasks** view backed by durable workspace records. Users and assistants manage the same tasks through the UI and `spark task` commands.

A task represents an intended outcome and may span multiple conversations and zero or more runs. Its progress is independent of run status. This delivery provides the foundation for future autonomous coordination; it does not add a scheduler or coordinator.

### Task records and lifecycle

- Use fixed stages: **Backlog → Planning → Ready → In progress → Review → Done**. Allow manual and assistant-driven movement, including returning to earlier stages.
- Store a stable ID, project identity, title, description, acceptance criteria, stage, priority, next action, timestamps, and revision.
- Keep **Blocked** and **Needs input** as independent indicators with explanatory text. Clearing an indicator does not change the stage.
- Support explicit links to conversations, repository artifacts, and runs. Run associations may identify the task stage they support; multiple runs can support one stage. Manual progress requires no run.
- Record task edits, stage changes, notes, and link changes in an activity history, including human/assistant attribution and conversation provenance when available.
- Require a completion note when moving to Done. It may reference validation, delivered changes, or manual review. Permit reopening and archiving; omit destructive deletion.
- Keep instructions and decisions in the task description and linked documents. A dedicated working-principles editor is deferred; existing project instructions remain applicable.

### Board and task details

- Add a Tasks tab for the active project, with one column per stage and cards showing title, priority, and attention indicators.
- Provide creation, editing, archiving, priority changes, and accessible stage selection. Defer drag-and-drop and custom ordering; sort by priority, then creation time.
- Open a detail panel containing the outcome, acceptance criteria, next action, indicators, related conversations/artifacts/runs, and activity history.
- Display linked runs’ actual statuses without automatically moving cards. Opening a run uses the existing run interface.
- Provide an **Needs attention** filter for blocked tasks, explicit task questions, and pending human questions in linked runs, including descendants. Route run answers through the existing question UI; do not duplicate answer storage.
- Refresh from the server after mutations, on tab activation, and through modest polling while Tasks is visible. Preserve unsaved edits and surface concurrent-change conflicts.

### Storage, APIs, and assistant integration

- Add task persistence using existing `spark-storage` atomic-write mechanisms under `SPARK_HOME`, scoped by project. Store each task and its activity together so an update cannot leave history inconsistent.
- Add a workspace task service and `/workspace/api/tasks` collection/item endpoints for listing, creating, reading, and updating tasks. Use revisions to reject stale updates rather than overwrite concurrent user or assistant changes.
- Validate project ownership and linked resource identities. Repository artifacts remain references to project-owned content, not copied sources of truth.
- Add `spark task list|get|create|update`, with structured JSON input/output and file/stdin support for multiline content. Updates cover fields, notes, stage changes, links, and archival. Reads return the revision required for mutations.
- Add an optional task reference to the existing conversation run-request path. Preserve it through approval and launch so the resulting run is durably associated with the task. Also allow explicit association of existing runs.
- Update bundled assistant guidance to inspect and maintain tasks through these commands, recording decisions, next actions, blockers, and completion evidence.
- Ready means the task is prepared for execution. Moving a card does not launch work or replace Spark’s existing run approvals.

### Verification and delivery boundaries

- Test persistence across restart, task/history consistency, stale-update rejection, invalid links, archival, reopening, and project isolation.
- Test zero-run manual tasks, multiple runs per task, many-to-many conversation links, and successful/failed runs leaving task stages unchanged.
- Test run-request association through approval and launch, including recovery without duplicate links.
- Test CLI/UI interoperability, accessible editing, filtering, attention routing, and concurrent edits.
- Run affected suites and the full `just test` gate.

Defaults: a personal queue with project-scoped records; stable identities and attribution leave room for future multi-user support. No accounts, permissions system, dependency scheduling, automatic Git delivery detection, cross-project dashboard, or autonomous task advancement in this delivery. Existing tasks are not inferred or imported automatically from change-request directories.
