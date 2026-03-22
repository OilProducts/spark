# Project vs Workspace Model Notes

This note captures the current model mismatch between Spark's intended architecture and the present implementation.

## Intended Model

- `workspace`: the global Spark layer rooted in `SPARK_HOME`
- `project`: one registered local repository/directory used as execution and artifact context
- `flows`: shared workspace/global authoring assets
- `launch policy`: workspace-global policy
- `trigger behavior`: workspace-global product policy unless explicitly designed otherwise
- `conversations`, `spec proposals`, `execution cards`, and relevant runs: project-scoped

## Major Mismatches and Correct Fixes

### 1. `ProjectScopedWorkspace` is the wrong abstraction

Status:
- complete on 2026-03-22

Current problem:
- The frontend uses `workspace` to mean per-project UI/session state.

Correct fix:
- Rename `ProjectScopedWorkspace` to `ProjectSessionState` or `ProjectViewState`.
- Rename `projectScopedWorkspaces` to `projectSessionsByPath` or equivalent.
- Keep only genuinely project-scoped state there:
  - `conversationId`
  - `projectEventLog`
  - `specId` / `specStatus` / `specProvenance`
  - `planId` / `planStatus` / `planProvenance`
- Remove `activeFlow` from that type entirely.

### 2. `activeFlow` is wrongly persisted per project

Status:
- complete on 2026-03-22

Current problem:
- Flow selection is restored from and written into per-project state.

Correct fix:
- Make `activeFlow` a top-level global UI/editor state value.
- Do not restore it from project-scoped state on project switch.
- Do not write it back into project-scoped state.
- Project switching should not implicitly clear or restore the selected flow unless the user is leaving a deleted or invalid flow.

### 3. Editor and Execution tabs are gated on active project selection

Status:
- complete on 2026-03-22

Current problem:
- The UI refuses to enter `editor` and `execution` without an `activeProjectPath`.

Correct fix:
- Remove navigation gating based on active project.
- Allow opening the Editor with no selected project.
- Allow opening the Execution view with no selected project, but gate run-start actions.
- Replace hard routing gates with local empty/disabled states such as:
  - "Select a flow to edit."
  - "Select a project to run this flow."

### 4. Flow create/delete/save is blocked on `activeProjectPath`

Status:
- complete on 2026-03-22

Current problem:
- Flow listing is global, but authoring mutations still require an active project.

Correct fix:
- Treat flows as workspace/global assets consistently.
- Creating a flow should not require an active project.
- Deleting a flow should not require an active project.
- Saving a flow should require only an active flow, not an active project.

### 5. Flow save baselines are keyed by project path

Status:
- complete on 2026-03-22

Current problem:
- Save baseline/conflict scope is keyed as `activeProjectPath::flowName`.

Correct fix:
- Key the save baseline by flow identity only.
- Minimal fix:
  - use `flowName`
- Better long-term fix:
  - use a backend-provided flow revision/etag if one exists later

### 6. Graph editing code treats project selection as a prerequisite

Status:
- complete on 2026-03-22

Current problem:
- Graph settings and node editing require both `activeProjectPath` and `activeFlow`.

Correct fix:
- Graph editing should require an active flow, not an active project.
- DOT-backed graph attr editing should autosave per flow.
- Node/edge edits should persist whenever a flow is open.
- Any project requirement should be limited to execution-related actions, not authoring.

### 7. Trigger bindings are still project-owned

Status:
- deferred to a separate trigger design pass

Current problem:
- Trigger-to-flow bindings still live on project records and under `/api/projects/flow-bindings`.

Correct fix:
- Stop storing trigger bindings on project records.
- If trigger routing remains configurable, move it to workspace-global config under `SPARK_HOME/config`.
- If some triggers are fixed product behavior, keep them as workspace-level defaults with optional global override only if needed.
- Treat trigger routing as workspace/product policy, not project identity.

Open design question:
- What should the trigger system actually be allowed to express?
- This needs a dedicated design pass before implementation changes.

### 8. Docs still mix "project-scoped" and "workspace-global"

Status:
- complete on 2026-03-22 for the project/workspace boundary cleanup
- trigger ownership wording remains intentionally unchanged pending item 7

Current problem:
- Specs and README still describe Spark in language that blurs workspace-global and project-scoped responsibilities.

Correct fix:
- Rewrite the core terminology so that:
  - Spark workspace = global layer over projects
  - project = execution/artifact context
  - flows = shared workspace assets
  - launch policy = workspace-global
  - trigger ownership stays on the existing project-scoped model until item 7 is redesigned
  - conversations/review artifacts/runs = project-scoped

## Recommended Sequence

1. Fix the frontend state model first:
   - mismatches 1, 2, 3, 4, 5, 6
2. Redesign and then fix trigger-binding ownership:
   - mismatch 7
3. Rewrite docs to match the corrected model:
   - mismatch 8

## Highest-Leverage First Change

If only one change happens first:

- remove `activeFlow` from project-scoped state

That is the central bad seam driving most of the current confusion.

## Trigger System Direction

The current `project -> trigger -> flow` model is too narrow for the intended use cases.

Examples that do not fit cleanly into project-owned trigger bindings:

- "every half hour between 09:00 and 17:00 on weekdays"
- "once a week on Mondays at 08:00"
- "every 5 minutes for the next 6 hours"
- poll email and, when a message matches criteria, launch another flow
- launch a follow-up flow in response to another flow result
- trigger from an external push/webhook

These are not all naturally project-owned. Some may target a project, but some are global workspace automations with no project at all.

### Correct Direction

Treat triggers as a workspace-owned automation system.

The model should be:

- `flows` are reusable programs
- `triggers` are workspace-owned automation definitions
- `project` is one possible execution context for a trigger, not the owner of trigger identity
- some triggers have no project scope at all

### Core Concepts

#### 1. TriggerDefinition

A workspace-global automation object with:

- stable id
- name
- enabled/disabled state

#### 2. TriggerSource

Trigger origin categories:

- `schedule`
- `poll`
- `webhook`
- `workspace_event`
- `flow_event`

#### 3. Scope

Optional execution scope:

- `project`
- `integration` or account-scoped context such as email
- `global`

Project is therefore a target/context option, not the universal owner.

#### 4. Condition

Filters or predicates such as:

- weekday and time window
- message importance or sender match
- previous flow outcome
- external payload fields

#### 5. Action

Usually:

- launch flow

But the action should support:

- target project when relevant
- launch context payload
- overrides and options
- cooldown/debounce semantics

#### 6. TriggerState

Runtime state should be tracked separately from definitions:

- last run
- next run
- dedupe/idempotency markers
- retry/backoff state
- recent failures

### Clean-Core Recommendation

Do not aim for "maximally flexible" v1 behavior.

Aim for an extensible core with a limited initial trigger set:

- schedule triggers
- polling triggers
- flow-completion or flow-event triggers
- webhook/push triggers

All of these should be workspace-owned.
Each may optionally specify a project target.

### Required Execution Controls

Any trigger system should also define:

- concurrency policy
- cooldown/debounce
- retry/backoff
- dedupe/idempotency
- enable/disable
- audit/history visibility

### Definition vs Runtime Ownership

Keep configuration and mutable runtime state separate.

Trigger definition/config should include:

- trigger id
- name
- source type
- source-specific config
- optional scope/target
- conditions
- action definition
- enabled/disabled state

Runtime state should include:

- last run
- next run
- in-flight status
- dedupe keys
- retry/backoff state
- recent execution history

Do not hide runtime state inside the trigger definition itself, and do not attach either of these to project records by default.

### V1 Guardrails

The first implementation should be intentionally smaller than the full design space.

Good v1 targets:

- recurring schedule triggers
- one-shot scheduled triggers
- polling triggers
- flow-event triggers
- webhook/push triggers
- optional project target on launch

Avoid in v1 unless a concrete use case forces it:

- deeply nested boolean condition trees
- arbitrary scripting inside trigger definitions
- many trigger-specific special cases in project records
- multiple ownership models for the same trigger

The architecture should be extensible, but the first implementation should stay legible.

### Implication for Mismatch #7

The fix for mismatch #7 is not merely moving `project.flow_bindings` elsewhere.

The real fix is:

- replace project-owned trigger bindings with a workspace-owned trigger subsystem
- treat project as an optional execution target
- treat flows as reusable actions rather than project-owned bindings
