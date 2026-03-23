# Project vs Workspace Model Notes

This note captures the current model mismatch between Spark's intended architecture and the present implementation.

## Intended Model

- `workspace`: the global Spark layer rooted in `SPARK_HOME`
- `project`: one registered local repository/directory used as execution and artifact context
- `flows`: shared workspace/global authoring assets
- `launch policy`: workspace-global policy
- `triggers`: workspace-owned automation definitions with optional project targets
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
- complete on 2026-03-22

Current problem:
- Trigger-to-flow bindings lived on project records and under `/api/projects/flow-bindings`.

Correct fix:
- Stop storing trigger bindings on project records.
- Store trigger definitions under `SPARK_HOME/config/triggers/<id>.toml`.
- Store runtime trigger state under `SPARK_HOME/workspace/trigger-state/<id>.json`.
- Expose workspace-owned triggers through `/workspace/api/triggers` plus shared webhook ingress at `/workspace/api/webhooks`.
- Use protected `workspace_event` triggers for the built-in approval/review hooks:
  - `spec_edit_approved`
  - `execution_card_approved`
  - `execution_card_rejected`
  - `execution_card_revision_requested`
- Support custom `schedule`, `poll`, `webhook`, and `flow_event` triggers.
- Treat project as an optional execution target on the trigger action, not the owner of trigger identity.
- Surface trigger administration in a first-class Triggers tab rather than project settings.
- Perform local/manual migration of the repo's existing project bindings into protected triggers; do not ship automatic migration logic.

### 8. Docs still mix "project-scoped" and "workspace-global"

Status:
- complete on 2026-03-22

Current problem:
- Specs and README still describe Spark in language that blurs workspace-global and project-scoped responsibilities.

Correct fix:
- Rewrite the core terminology so that:
  - Spark workspace = global layer over projects
  - project = execution/artifact context
  - flows = shared workspace assets
  - launch policy = workspace-global
  - triggers = workspace-owned automation definitions with optional project targets
  - conversations/review artifacts/runs = project-scoped

## Recommended Sequence

1. Fix the frontend state model:
   - completed on 2026-03-22 for mismatches 1, 2, 3, 4, 5, 6
2. Replace project-owned trigger bindings with workspace-owned triggers:
   - completed on 2026-03-22 for mismatch 7
3. Rewrite docs to match the corrected model:
   - completed on 2026-03-22 for mismatch 8

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

### Implemented V1 Shape

The implemented v1 trigger subsystem keeps the core intentionally small:

- workspace-owned trigger definitions
- separate persisted runtime state
- `launch_flow` as the only action type
- optional project target on the action
- protected system triggers plus custom triggers

Supported trigger sources in v1:

- `workspace_event`
- `schedule`
- `poll`
- `webhook`
- `flow_event`

### Implemented Runtime Controls

The implemented controls are:

- single-flight concurrency per trigger
- source-specific dedupe markers
- enabled/disabled state
- persisted last result / last error / next run visibility
- recent execution history

V1 intentionally does not add a general retry/backoff or cooldown layer beyond normal schedule/poll cadence.

### Definition vs Runtime Ownership

Definitions and runtime state are stored separately:

- definitions in `SPARK_HOME/config/triggers`
- runtime state in `SPARK_HOME/workspace/trigger-state`

This keeps mutable runtime bookkeeping out of project records and out of the trigger definition file itself.

### Shared Webhook Ingress

Webhook triggers use one shared ingress endpoint:

- `POST /workspace/api/webhooks`

Each webhook trigger owns:

- `webhook_key` for routing
- `webhook_secret` for authentication

The webhook payload becomes launch context input for the trigger-fired flow.

### Implication for Mismatch #7

The completed fix for mismatch #7 is:

- replace project-owned trigger bindings with a workspace-owned trigger subsystem
- treat project as an optional execution target
- treat flows as reusable actions rather than project-owned bindings
- move trigger administration into a first-class workspace UI surface
