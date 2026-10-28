# Spark Flow Authoring Guide

A Spark flow is a directory with one markdown file per node, following the
Open Knowledge Format (OKF) v0.2: each file has YAML frontmatter, a markdown
body, and a required `type`. The flow's name is the directory's path relative
to the flows root, with no extension, for example `examples/simple-linear`.
YAML flow files are not loaded.

## Where Flows Live

Flows, roles and playbooks resolve through two roots, in order:

1. The project's org repo, `<project>/.spark/agent-org/` (`flows/`, `roles/`
   and `playbooks/`). It is a git repository; its history is the audit trail.
2. The bundled library, `$SPARK_HOME/bundled/` (`flows/`, `roles/` and
   `playbooks/`). Spark
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

Flows may keep project scratch under `<project>/.spark/` (for example
`checkouts/` or `changes/`), but must not use the data directory's top-level
names there: `attractor`, `bundled`, `config`, `logs`, `playbooks`, `runtime`
or `workspace`. The Home project's directory is `$HOME`, so its `.spark/` is
the default data directory itself.

The Rust `FlowDefinition` type is the assembled form of a flow, and
`assets/schemas/flow-definition.schema.json` describes its JSON form. The
editor's structured mode loads and saves that form as JSON, and the server
writes it into the directory, rewriting only the files that changed. The
editor's file view shows and saves the flow's files with their exact text.

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
- `defaults`: common model, provider, reasoning, and retry defaults.
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

A role may set `isolated: true`. Its nodes always run in a fresh session: a
node using an isolated role may not set `runtime.session`. Reviewers and
verifiers use isolated roles so they never inherit the author's reasoning.

A role that may run as a project's standing role sets `standing: true`, and
may set its launch limits under `budget` (`concurrent_runs`, default 4;
`runs_per_day`, default 20). `budget` without `standing: true` fails
validation. See the operations guide's standing roles section.

## Sessions

Every `agent_task` node runs in a fresh provider session unless it sets
`runtime.session: <name>` (letters, digits, `_`, or `-`). Nodes with the same
session name continue one provider session, including a node re-entered in a
loop: the node's prompt, with its role persona and frame, is sent as the next
turn of that conversation.

```yaml
runtime:
  session: implementer
```

- In a root run, a session belongs to the run. In a child run, it belongs to
  the parent's subflow node: re-entering the same subflow node continues the
  child's sessions, and different subflow nodes never share one. Continues,
  retries and crash recovery keep sessions.
- Claude Code resumes the session (`--resume`), Codex resumes a persistent
  thread, and native providers reload the saved history from
  `<run dir>/sessions/<scope>/<name>.json`. When a stored session cannot be
  resumed, the node starts a fresh one and records a `continuity_reset` event.
- Nodes sharing a session may not set different `llm_provider` or `llm_model`
  values, and a node whose resolved provider differs from the one its session
  started with fails. Two nodes may not run in one session at once, so do not
  share a session across parallel branches.
- `session` is only allowed on `agent_task` nodes. The former `fidelity` and
  `thread_id` settings are rejected.

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
  "spark_version": "<version>",
  "sessions": {"implementer": ["work"]}
}
```

`org` is null when the run has no org repo. `sessions` lists the nodes of each
named session in the flow as launched, and is omitted when there are none. Launches, `flow_name` continues and
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

The child starts from the parent's context, with `config.input_map`
(`child_key: parent_key`) copying parent values into child keys. To hand child
values back, `config.output_map` (`child_key: parent_key`) copies each listed
child key into the parent when the child run ends, whether it succeeded or
failed; a listed child key that is absent clears the parent key. Every
`parent_key` must be listed in the subflow node's `contracts.writes_context`.
`context.stack.child.*` reports the child's status as before.

```yaml
config:
  flow_ref: workers/implement-task
  output_map:
    context.task.summary: context.implementation.summary
contracts:
  writes_context:
  - context.implementation.summary
```

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

## Loops and Visit Limits

A visit is one entry into a node through routing. Every node may be visited
32 times per run; `runtime.max_visits` sets its own limit. Counts cover the
whole run: crash recovery, retries and continues keep them. A child run counts
its own nodes, and each parallel branch execution counts its nodes from zero.

Routing never selects a route whose target has used all its visits; it moves
on to the node's next eligible route, retry target or fallback. An agent's
route list marks such a route `(unavailable: visit limit reached)`. When no
other route remains, the node fails with
`visit_limit_reached: <target> used all <N> visits; no other route from <node>`,
whatever its outcome. This gives two loop patterns without counter nodes:

- **Fail at the limit**: the loop has no alternative route. Validation fails
  into a diagnosis node limited to three visits; the fourth failure fails the
  run.

  ```yaml
  # validate.md
  routes:
  - to: diagnose_validation
    label: Fix
    condition: outcome=fail
  - to: finalize
    condition: outcome=success
  # diagnose_validation.md
  runtime:
    max_visits: 3
  ```

- **Continue at the limit**: the loop has another route. A review that allows
  one re-review sends its author once more, then takes the other route.

  ```yaml
  # review.md
  routes:
  - to: revise
    label: Revise
  - to: done
    label: Accept
  # revise.md
  runtime:
    max_visits: 1
  ```

  Once `revise` is used, a `Revise` answer falls through to `Accept`.

### Author and Isolated Reviewers

The bundled `software-development/design-change` flow combines sessions,
isolated roles and a visit limit into a review loop:

- `draft` and `resolve` use the `author` role and share `session: designer`,
  so the author keeps everything it learned while drafting.
- A `parallel` node, `review`, fans out to reviewers that use the isolated
  `adversarial-reviewer` role. Each reviews through one lens and reads only
  the document's path, never the request or the author's reasoning: the
  document has to stand on its own. Each reviewer writes its complete
  findings to `context.review.lens_findings`; its branch's context, and so
  that key, lands in `parallel.results`. A response itself reaches later
  nodes only as `last_response`, cut to 200 characters.
- `review` also routes to `fan_in` with `weight: 1`. After the fan-out,
  routing takes a `parallel` node's own edges like any node's, and the
  heaviest edge sends it to the join instead of rerunning a reviewer.
- `synthesize`, after the `fan_in`, merges the reviewers' findings from
  `parallel.results` into one list without judging them again.
- `resolve` fixes each finding, contests it with checkable evidence, or asks
  the owner, then answers `Re-review` (back to `review`) or `Done`.
  `review` sets `max_visits: 2`, so once the re-review is used, `Re-review`
  shows as unavailable and routing takes `Done`.

The approach is adapted from the elephant/goldfish model
([google/elephant-goldfish-model](https://github.com/google/elephant-goldfish-model),
Apache-2.0): an author who holds the context, reviewers who see only the
document, findings settled by fix, contest or escalation, and a single
re-review.

## Validation

Validate a flow directory before launching:

```bash
spark flow validate --dir crates/spark-assets/assets/flows/examples/simple-linear --text
```

Validation assembles the directory into a `FlowDefinition`, applies semantic
rules, and then verifies the runtime graph derived from the typed model.
A file inside `<project>/.spark/agent-org` resolves roles through that
project's org, then the bundled library. `spark flow list|describe|get|validate`
take `--project <path>` to include a project's org flows.
