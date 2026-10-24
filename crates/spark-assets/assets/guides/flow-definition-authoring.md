# Spark Flow Authoring Guide

A Spark flow is a directory with one markdown file per node, following the
Open Knowledge Format (OKF) v0.2: each file has YAML frontmatter, a markdown
body, and a required `type`. The flow's name is the directory's path relative
to the flows root, with no extension, for example `examples/simple-linear`.
YAML flow files are not loaded.

## Where Flows Live

Flows and roles resolve through two roots, in order:

1. The project's org repo, `<project>/.spark/agent-org/` (`flows/` and
   `roles/`). It is a git repository; its history is the audit trail.
2. The bundled library, `$SPARK_HOME/bundled/` (`flows/` and `roles/`). Spark
   rewrites it from its packaged assets on every start, so never edit it.

A project file with the same name as a bundled one replaces it. A launch with
no project uses bundled flows only, and a project without an org repo sees
the bundled library. Every flow can be launched, by people and by agents.

Editor saves write into the project's org repo, creating it on the first
write (with `<project>/.spark/.gitignore` holding `*`), and commit only the
files that changed with the message `Edit <flow> in the flow editor`, under
the repo's normal git identity. Saving a bundled flow first copies it into the
org. Deleting a project copy reverts to the bundled flow; bundled flows cannot
be deleted.

The Rust `FlowDefinition` type is the assembled form of a flow, and
`assets/schemas/flow-definition.schema.json` describes it. The editor loads and
saves that assembled form as one YAML document; the server splits it into the
directory on save.

## Minimal Flow

```
examples/minimal/
  flow.md
  start.md
  plan.md
  done.md
```

`flow.md`:

```markdown
---
type: Flow
id: simple_linear
title: Simple Linear Workflow
description: Plan, implement, and summarize a small change.
schema_version: '1'
---

Inspect the repository, make one targeted improvement, and summarize it.
```

`start.md`:

```markdown
---
type: start
title: Start
routes:
- to: plan
---
```

`plan.md`:

```markdown
---
type: agent_task
title: Plan
routes:
- to: done
---

Inspect the repository and plan the work.
```

`done.md`:

```markdown
---
type: exit
title: Done
---
```

## flow.md

`type: Flow`, plus the flow-level fields:

- `id`: stable flow id, using letters, digits, `_`, or `-`.
- `title` and `description`: catalog metadata.
- `schema_version`: current value is `"1"`.
- `inputs`: launch inputs with `key`, `label`, `type`, `description`, `required`,
  and optional `default`.
- `defaults`: common model, provider, reasoning, fidelity, and retry defaults.
- `metadata` and `extensions`: compatibility and experimental metadata.

The body is the flow's goal.

## Node Files

Each node is `<node id>.md`. Node ids use letters, digits, `_`, or `-`; `flow`,
`index`, and `log` are reserved.

- `type` is the node kind: `start`, `exit`, `agent_task`, `human_gate`,
  `conditional`, `parallel`, `fan_in`, `tool`, or `subflow`.
- `title` is the node label.
- `description`, `config`, `context`, `runtime`, `contracts`, `manager`,
  `retry`, `execution`, `ui`, and `extensions` keep the `FlowNode` field names
  and shapes. `config` holds the kind's settings without `kind` (the `type`
  sets it) and without the field the body carries.
- `routes` lists the node's outgoing transitions in order. Each route has `to`
  and optional `label`, `condition`, `weight`, `transition`, and `extensions`.
- `role` names a role file for an `agent_task` node (see Roles).

The body depends on the kind:

- `agent_task` and `human_gate`: the prompt.
- `exit`: its `result_summary_prompt`.
- `tool`: exactly one fenced code block holding the command.
- Every other kind: empty. The loader rejects text it would otherwise ignore.

`index.md` and `log.md` are OKF-reserved and ignored. Any other file or
directory, hidden ones included, is an error.

## Roles

A role is `roles/<name>.md` in the org repo or the bundled library, with
`type: Role`. Its body is a
persona; its frontmatter may set `execution` with the same fields as a node's
`execution`.

```markdown
---
type: Role
---

You are an adversarial reviewer. ...
```

An `agent_task` node sets `role: <name>`. Unknown roles, and `role` on any other
kind, fail validation. When a flow loads for a run, the role body is prepended
to the node's prompt, followed by a blank line, and the role's `execution` is
merged under the node's own field by field: node values win. The merged values
then behave like node settings, so they also beat launch-time model and
reasoning overrides. The editor shows the authored form (`role` set, prompt
without role text); runs execute and snapshot the resolved form.

## Execution Locks

`flow.md` may declare an execution lock. Runs whose locks resolve to the same
identity queue and execute one at a time:

```yaml
execution_lock:
  scope: project
  key: software-development-integration
  conflict_policy: queue
```

`scope` must be `project`: identity comes from the repository containing the
run's working directory, so linked git worktrees share one lock. `key` must be
non-empty. `conflict_policy` is `queue`, the default. Child runs inherit their
parent's lock.

## Run Manifests

Each run records the files it used in `artifacts/flow/files.json`:

```json
{
  "files": {"org/flows/<flow>/<node>.md": "<sha256>", "bundled/roles/<name>.md": "<sha256>"},
  "org": {"commit": "<org HEAD>", "dirty": false},
  "spark_version": "<version>"
}
```

`org` is null when the run has no org repo. Launches, `flow_name` continues and
subflow children compute their own manifest; snapshot continues keep the
source run's.

## Tool Context Bindings

Tool nodes compose with context through two optional `config` maps:

- `env_map` binds declared context values into named environment variables, so
  commands consume context without shell interpolation. Every binding must
  resolve to a non-null context value or the node fails before the command runs.
- `output_map` parses the command's stdout as JSON and maps dotted paths into
  declared context keys. Commands used with `output_map` must print only JSON
  on stdout (redirect noisy tooling to `/dev/null` or stderr). Mapped keys
  outside `context.tool.*` must be declared in `contracts.writes_context`.

`prepare_workspace.md`:

````markdown
---
type: tool
title: Prepare Workspace
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
  output_map:
    context.workspace.path: path
contracts:
  writes_context:
  - context.workspace.path
routes:
- to: implement
  condition: outcome=success
---

```sh
repo_root=$(git rev-parse --show-toplevel)
mkdir -p "${repo_root}/.spark/checkouts"
printf '*\n' > "${repo_root}/.spark/.gitignore"
worktree="${repo_root}/.spark/checkouts/${SPARK_RUN_ID}"
git worktree add -b "spark/task/${SPARK_RUN_ID}" "$worktree" >/dev/null
printf '{"path":"%s"}' "$worktree"
```
````

## Run Result Summaries

An exit node may opt in to producing the run's result:

```markdown
---
type: exit
title: Done
config:
  result_summary: true
---
```

When the run reaches an opted-in exit — or fails, in which case any opted-in
exit counts — the runtime executes one summarizer agent whose working
directory is the run's artifact root. It reads the recorded transcripts
(`events.jsonl`, `checkpoint.json`, `logs/<node>/…`) directly and its markdown
response becomes the run result shown in the Runs view. The exit node's body,
when present, overrides the default instructions. A summarizer failure never
fails the run; the result falls back to the exit predecessor's response with
the error recorded.

## Subflows

A `subflow` node's `config.flow_ref` names another flow directory. It resolves
relative to the directory that contains the parent flow's directory, so
`software-development/implement-change` refers to
`software-development/workers/implement-task` as `workers/implement-task`.
The resulting name resolves through the roots like any flow, so a project copy
of a bundled child replaces it.

A subflow's child run normally inherits the parent's working directory, or an
authored static `manager.child_workdir`. To bind the child to a directory
produced by an earlier node, use `manager.child_workdir_from` with a context
key. The runtime validates that the resolved directory exists and is either
inside the run's working directory or a linked git worktree of the same
repository. Declaring both `child_workdir` and `child_workdir_from` fails the
node. Run working directories are immutable once a run starts; crossing a
workspace boundary always means launching a child run bound to that workspace.

`implement.md`:

```markdown
---
type: subflow
title: Implement
config:
  flow_ref: workers/implement-task
manager:
  child_workdir_from: context.workspace.path
routes:
- to: evaluate
---
```

## Conditions

Route conditions use the runtime expression subset, such as:

```yaml
condition: outcome=success
condition: outcome=fail && preferred_label=Rewrite
```

## Validation

Validate a flow directory before launching:

```bash
spark flow validate --file crates/spark-assets/assets/flows/examples/simple-linear --text
```

Validation assembles the directory into a `FlowDefinition`, applies semantic
rules, and then verifies the runtime graph derived from the typed model.
A file inside `<project>/.spark/agent-org` resolves roles through that
project's org, then the bundled library. `spark flow list|describe|get|validate`
take `--project <path>` to include a project's org flows.

## Converting YAML Flows

Older single-file YAML flows convert with the same writer the editor uses:

```bash
cargo run -p attractor-dsl --example convert-yaml-flows -- <yaml-root> <flows-dir>
```

It skips flows whose directory already exists and drops the extension from
subflow `flow_ref`s.
