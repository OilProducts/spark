# Storage Boundaries

This document defines which data belongs to Spark Spawn and which data belongs to the project being worked.

It exists to keep ownership explicit and to prevent Spark Spawn operational state from being confused with project source material.

## Core Rule

Spark Spawn owns workflow state.

The project owns its canonical content and outputs.

In practice:
- Spark Spawn state lives under `SPARKSPAWN_HOME`
- project-owned artifacts live in the project repository

The default `SPARKSPAWN_HOME` is:
- `~/.sparkspawn`

## `SPARKSPAWN_HOME` Layout

`SPARKSPAWN_HOME` is the authoritative home for Spark Spawn operational state.

Its canonical structure is:

```text
SPARKSPAWN_HOME/
  config/
  runtime/
  logs/
  projects/
    <project-id>/
      project.toml
      conversations/
      workflow/
      proposals/
      execution-cards/
      runs/
```

### Top-Level Directories

- `config/`
  Global Spark Spawn user configuration, defaults, and prompt customization.

- `runtime/`
  Spark Spawn machine-local runtime state such as active session bookkeeping and other non-project operational state.

- `logs/`
  Spark Spawn application logs.

- `projects/`
  Project-scoped Spark Spawn workflow state, keyed by stable Spark Spawn storage id.

## Project Identity

A registered project has two co-equal identity attributes:
- canonical absolute project path
- internal Spark Spawn `project-id`

The canonical absolute path is the user-facing project identity.

The internal `project-id` is a Spark Spawn storage identifier used to name directories and indexes under `SPARKSPAWN_HOME`.

`project.toml` is the source of truth for the mapping between these identity attributes.

It should contain at least:
- canonical project path
- display name
- created timestamp
- last opened timestamp

Path moves are out of scope.

## Per-Project Spark Spawn Layout

Each registered project has a directory under:
- `SPARKSPAWN_HOME/projects/<project-id>/`

This is Spark Spawn-owned state for that project.

It contains:

- `project.toml`
  Canonical Spark Spawn metadata for the registered project.

- `conversations/`
  Conversation threads, turn history, turn events, and resumable AI-session metadata.

- `workflow/`
  Workflow event logs and other project-scoped orchestration state.

- `proposals/`
  Spec proposal records and related review state.

- `execution-cards/`
  Execution card records and related review state.
  This is also the home for the work-package/task-tracker planning surface; there is no separate durable implementation-plan artifact category.

- `runs/`
  Run metadata and Spark Spawn-managed run records for that project.

## Spark Spawn-Owned Data

The following are Spark Spawn-owned and belong under `SPARKSPAWN_HOME`:
- conversation threads
- conversation turn/event history
- workflow logs
- spec proposals
- execution cards
- task/work-package planning state
- run metadata
- resumable AI session state
- project registry metadata
- user configuration and defaults

These records may refer to a project, but they are still owned by Spark Spawn.

They must not be written into a project repository.

## Project-Owned Data

Project-owned data belongs in the repository for that project.

This includes:
- the actual specification
- user stories
- intent documents
- code
- tests
- generated outputs that are part of the durable project record

If the project is expected to retain the artifact independently of Spark Spawn, it belongs in the project repository.

## Canonical Repo Structure

Repo-owned material should be organized by content, not by tool.

There should not be a generic repo-level `sparkspawn/` directory used as a catch-all.

The canonical home for specification material inside the repository is:
- `specs/`

`specs/` should contain canonical project documents such as:
- specifications
- user stories
- intent/design records
- other project-owned planning or behavioral reference material

Code should remain in the repository's normal source directories.

Tests should remain in the repository's normal test directories.

If a new project-owned artifact class is added later, it should be placed in the domain-appropriate repo location, not a generic Spark Spawn folder.

## Borderline Artifact Decisions

Some artifacts are easy to misclassify. Use these defaults:

| Artifact | Owner | Canonical location | Durable? |
| --- | --- | --- | --- |
| Conversation thread | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/conversations/` | Yes |
| Turn/event history | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/conversations/` | Yes |
| Workflow log | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/workflow/` | Yes |
| Spec proposal | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/proposals/` | Yes |
| Execution card / work package | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/execution-cards/` | Yes |
| Run metadata | Spark Spawn | `SPARKSPAWN_HOME/projects/<project-id>/runs/` | Yes |
| Implementation plan summary | Spark Spawn | attached to execution-card/task-tracker state | No separate artifact class |
| Specification | Project | repository `specs/` | Yes |
| User story | Project | repository `specs/` | Yes |
| Intent/design document | Project | repository `specs/` | Yes |
| Code | Project | repository source tree | Yes |
| Tests | Project | repository test tree | Yes |
| Generated output intended as project record | Project | domain-appropriate repo location | Yes |

## Cross-Project Behavior

Because Spark Spawn owns workflow state centrally:
- a conversation about `~/collatz` belongs in `SPARKSPAWN_HOME/projects/<project-id>/...`
- a conversation about this repository also belongs in `SPARKSPAWN_HOME/projects/<project-id>/...`

The invariant is not “store project workflow state inside the target repository.”

The invariant is:
- Spark Spawn workflow state stays inside `SPARKSPAWN_HOME`
- project-owned source artifacts stay inside the project repository

## Invariants

The system must maintain these invariants:

- Spark Spawn operational workflow state is authoritative in `SPARKSPAWN_HOME`
- project source artifacts are authoritative in the project repository
- opening one project must not write Spark Spawn workflow state into any project repository
- repository structure for project-owned artifacts is content-shaped, not tool-shaped

## Implementation Guidance

When deciding where new data belongs, ask:

1. Is this Spark Spawn-managed workflow state?
2. Is this project-owned source content?
3. Is this an operational record, or part of the project's durable record?
4. If Spark Spawn disappeared, would the project still need this artifact in its repository?

If it is Spark Spawn-managed workflow state, default to `SPARKSPAWN_HOME`.

If it is project-owned content or a durable project output, default to the project repository.
