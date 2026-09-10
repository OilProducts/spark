# Simplify tasks to a minimal, visually integrated six-stage board

## Summary

Keep **Backlog, Planning, Ready, In progress, Review, and Done**. Reduce tasks to a required title, optional description, and stage. Preserve archive/restore and collapsed activity history.

Make the Tasks tab feel like a finished part of Spark by using the existing application’s layout, typography, surfaces, and controls.

This is an initial buildout with no existing tasks, so no data migration or compatibility layer is needed.

## User experience and visual design

- New task pane contains **Title**, **Description**, **Stage** (default Backlog), **Create task**, and **Cancel**. No prescribed description template.
- Existing tasks use the same three fields, with Save, Close, and Discard changes. Offer Archive/Restore as an action outside the editing fields.
- Remove priority, next action, separate acceptance criteria, blockers/questions, relationship editors, related-resource sections, and note/completion-evidence entry.
- Keep activity collapsed by default. Show human-readable field changes, actor, and timestamp rather than raw JSON. Remove UUID/revision display from the normal pane.
- Keep six board columns, title-only cards, and Show archived. Remove priority labels, attention indicators, and Needs attention filtering.
- Match the Runs view and its inspector for toolbar density, heading hierarchy, pane boundaries, spacing, and action placement. Keep the board as the main workspace and the editor as its detail pane.
- Reuse Spark’s shared buttons, inputs, textarea, select, and disclosure patterns. Use existing theme tokens for backgrounds, borders, text, hover, selection, and focus; introduce no separate task-specific palette or visual system.
- Style cards, column headings, empty states, loading states, and errors consistently with surrounding Spark views. Remove persistent implementation-oriented helper text.
- Preserve keyboard access, focus restoration, responsive layout, draft preservation, save-error handling, and concurrent-edit protection. Present conflicts in readable UI rather than raw record dumps.

## Model, API, and assistant changes

- Reduce task fields to `title`, `description`, `stage`, and `archived`. Retain task/project identity, timestamps, revision, and attributed activity history.
- Remove the unused fields and relationship types throughout storage, service, frontend, CLI examples, and tests. Keep validation for required titles, valid stages, project scope, and revision conflicts.
- Allow creation or movement into Done without a completion note. All six stages remain manually or assistant editable; stage changes launch nothing.
- Retain optional API activity notes and actor attribution, but remove conversation provenance and resource associations. No note is required.
- Remove task references from conversation run requests, the `--task-id`/`--task-stage` flags, task launch metadata injection, and run reconciliation. Existing run approval and execution behavior remains intact.
- Task listing returns `{ "tasks": [...] }`, without runs or attention data, and no longer reads run state. Sort by creation time ascending, then ID.
- Keep `spark task list|get|create|update`; update bundled guidance to use title, description, stage, and archive status. Assistants record useful issue detail in the description without mandatory sections.
- Modify existing components and services; add no dependencies, scheduler, drag-and-drop, or execution controls.

## Verification

- UI tests cover title-only creation, optional description, Backlog default, all stage changes, Done without a note, reopening, archive/restore, and collapsed activity.
- Adapt existing draft, project-switching, pending-save, conflict, and accessibility tests to the reduced fields.
- Service/API/CLI tests cover minimal records, persistence, history attribution, project isolation, stale revisions, and rejection of removed fields and flags.
- Verify ordinary conversation run requests still follow approval and launch successfully without task integration.
- Visually compare Tasks with Runs in supported light and dark themes at desktop and narrow widths. Check empty and populated boards, long titles, the editor, keyboard focus, and error/conflict states. Visual consistency is part of acceptance, beyond merely using shared components.
- Run affected Rust and frontend tests, frontend build, and the repository’s `just test` gate.

## Defaults

- No migration: there are no existing task records to preserve.
- Description remains plain editable issue text.
- Archive is the housekeeping action; destructive deletion is outside this change.
- Tasks remain explicitly managed records. No automatic stage advancement or workflow launching is added.
- Styling changes are limited to Tasks; reuse existing Spark conventions without redesigning other tabs.
