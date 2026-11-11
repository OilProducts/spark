# Spark Flow Authoring Guide

A Spark flow is a directory with one markdown file per node, following the
Open Knowledge Format (OKF) v0.2: each file has YAML frontmatter, a markdown
body, and a required `type`. The flow's name is the directory's path relative
to the flows root, with no extension, for example `examples/simple-linear`.
YAML flow files are not loaded.

## Writing Principles

Apply these whenever you write or change a flow or role.

1. **Trust the agent with the method.** Say what outcome you want and how you'll know it's done. Leave the method to the agent unless getting it wrong would be costly.

2. **Match the guard to the real risk.** A risk is real when you can name what triggers it and what it costs. A guard has a cost too, so don't guard against risk the work doesn't carry.

3. **Keep the record in the run.** The run is the audit trail. Produce artifacts only when the artifact is the deliverable.

4. **Separate making from checking.** Whoever produces the work shouldn't be the one who accepts it. Checks should try to disprove claims, including the unstated claim that nothing else broke, not add work of their own.

5. **Hold scope.** Do what the request needs and don't build beyond it. This applies to authors and reviewers alike.

6. **Stop honestly.** When something needs human judgment, stop and say so instead of guessing, and never quietly lower a bar to get to done.

## Where Flows Live

Flows, roles and playbooks resolve through two roots, in order:

1. The project's definitions, `<project>/.spark/definitions/` (`flows/`,
   `roles/` and `playbooks/`). Registering a project writes nothing there;
   the project owns whatever files it adds.
2. The bundled library, `$SPARK_HOME/bundled/` (`flows/`, `roles/` and
   `playbooks/`). Spark rewrites it from its packaged assets on every start,
   so never edit it. It also holds `guides/`, this guide among them, as
   read-only references; guides take no part in project lookup.

A project file with the same name as a bundled one replaces it. A launch with
no project uses bundled flows only, and a name missing from the project's
definitions falls back to the bundled library. Every flow can be launched, by people and by agents.

To choose models for one project's flow or nodes, copy the flow into the
project's definitions and set `defaults` or the nodes' `execution` there. No
project settings file adds another layer: a project's own default model and
execution profile are private to the installation, and launch-time choices
apply as usual. The operations guide's storage section lists where Spark keeps
everything else.

Editor saves write the changed files into the project's definitions; Spark
runs no Git there, and versioning them is the operator's choice. Saving a
bundled flow first copies it into the definitions. Deleting a project copy
reverts to the bundled flow; bundled flows cannot be deleted.

Flows keep durable working data (checkouts, ledgers, generated scripts) in
the project's private work area, outside the project: Spark supplies it to
every run of the project, root and child alike, as the context binding
`internal.project_work_dir` and to every tool as `SPARK_PROJECT_WORK_DIR`.
Each flow uses its own subdirectory, such as `checkouts/` or
`spec-implementation/`. An agent node that needs the path declares
`internal.project_work_dir` in `contracts.reads_context`. The area outlives
runs and project removal; Spark never cleans it, so a flow that creates
something there removes it when it is done with it. Write nothing for this
into the project's `.spark/` and no ignore file. A Git worktree there still
belongs to its source repository: it stays usable only while that repository
does, and moving it breaks it.

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
  failed.md
```

`flow.md`:

```markdown
---
type: Flow
id: simple_linear
title: Simple Linear Workflow
description: Plan, implement, and summarize a small change.
schema_version: '2'
---

Inspect the repository, make one targeted improvement, and summarize it.
```

`start.md`:

```markdown
---
type: start
title: Start
routes:
  next:
    to: plan
    description: Enter the first workflow step.
---
```

`plan.md`:

```markdown
---
type: agent_task
title: Plan
contracts:
  response: decision_envelope
routes:
  done:
    to: done
    description: The plan is complete and recorded.
  unfinished:
    to: plan
    description: Required planning work remains and can be continued in the same session.
  blocked:
    to: failed
    description: Planning cannot complete without unavailable input or authority; record the blocker in notes.
---

Inspect the repository and plan the work. Choose `done` once the plan is
complete. Respond with ONLY a JSON decision envelope.
```

`done.md`:

```markdown
---
type: exit
title: Done
config:
  result:
    name: completed
    outcome: success
---
```

`failed.md`:

```markdown
---
type: exit
title: Failed
config:
  result:
    name: blocked
    outcome: failure
---
```

## flow.md

`type: Flow`, plus the flow-level fields:

- `id`: stable flow id, using letters, digits, `_`, or `-`.
- `title` and `description`: catalog metadata.
- `schema_version`: required; the only accepted value is `'2'`. A missing
  version or `'1'` is rejected.
- `inputs`: launch inputs with `key`, `label`, `type`, `description`, `required`,
  and optional `default`.
- `defaults`: common model, provider, reasoning, and retry defaults.
- `metadata` and `extensions`: compatibility and experimental metadata.

The body is the flow's goal.

## Node Files

Each node is `<node id>.md`. Node ids use letters, digits, `_`, or `-`; `flow`,
`index`, and `log` are reserved.

- `type` is the node kind: `start`, `exit`, `agent_task`, `human_gate`,
  `switch`, `parallel`, `fan_in`, `tool`, or `subflow`.
- `title` is the node label.
- `description`, `config`, `context`, `runtime`, `contracts`, `manager`,
  `retry`, `execution`, `ui`, and `extensions` keep the `FlowNode` field names
  and shapes. `config` holds the kind's settings without `kind` (the `type`
  sets it) and without the field the body carries.
- `routes` maps each decision the node can produce to its transition (see
  Decisions and Routes). `exit` and `parallel` nodes have no routes.
- `role` names a role file for an `agent_task` node (see Roles).

The body depends on the kind:

- `agent_task` and `human_gate`: the prompt.
- `exit`: its `result_summary_prompt`.
- `tool`: exactly one fenced code block holding the command.
- Every other kind: empty. The loader rejects text it would otherwise ignore.

`index.md` and `log.md` are OKF-reserved and ignored. Any other file or
directory, hidden ones included, is an error.

## Decisions and Routes

A node that completes produces exactly one decision, and `routes` declares
every decision it may produce. `routes` is a YAML map, never a list:

```yaml
routes:
  ready:
    to: validate
    description: The review is complete and no blocking issue remains under this node's stated acceptance criteria.
  fix:
    to: implement
    description: The completed review found concrete defects requiring another implementation pass; record actionable findings.
```

- Each key is a decision ID matching `[a-z][a-z0-9_]*`; duplicate keys are a
  parse error.
- Each entry has `to`, an existing node id, and a nonempty `description`.
  Optional `label` is display text and optional `extensions` is presentation
  metadata; neither affects routing. Several decisions may target one node.
- There are no conditions, weights, fallback or default routes. The runtime
  looks the decision up in the map; a decision missing from the map fails the
  run with `no_matching_route`.

Where each decision comes from depends on the kind:

| Kind | Decision |
|---|---|
| `start` | Built-in `next`, its only route. |
| `agent_task` | The agent's decision envelope. |
| `tool` | The exit-code map or the JSON decision field (see Tool Decisions). |
| `switch` | A context value compared with declared cases (see Switches). |
| `human_gate` | The person's choice; the routes are the choices. |
| `parallel` | None; the fork enters its join (see Parallel Branches). |
| `fan_in` | Built-in `next`, its only route. |
| `subflow` | The child's result, mapped through `config.results` (see Subflows). |
| `exit` | None; it ends the run with its declared result. |

### Agent Decisions

An `agent_task` sets `contracts.response: decision_envelope`. The agent
answers with one JSON object:

```json
{"decision": "ready", "context_updates": {"context.review.summary": "..."}, "notes": "..."}
```

Only `decision` is required. `context_updates` follows the node's
`contracts.writes_context`, and `notes` carries the evidence for the decision.
Any other key, a missing or undeclared decision, or malformed JSON is a
contract error; the runtime asks the agent to repair it within the bounded
repair attempts, then fails the node. Plain text never counts as a decision.

The runtime appends the response contract and the node's decisions, each with
its target and description, to the prompt. Write the prompt around the
decision IDs, for example "choose `fix` when the review found blocking
defects", and keep its acceptance criteria and context contract.

A negative judgment such as `fix`, `rejected` or `gaps` is a completed
execution with an ordinary decision. Give every agent an honest inability
decision, conventionally `blocked` to a failure exit, and distinguish it from
a completed review that found defects. A bounded step with work genuinely left
may choose `unfinished` back to itself.

### Execution Errors

Provider and transport errors, spawn failures, timeouts and exhausted response
repair are execution errors, never decisions: no route handles them. Retryable
errors retry under the node's retry settings; otherwise the node fails and the
run fails with its evidence. Long-running work stays inside the invocation:
the agent waits for and polls the commands it starts instead of answering
early.

## Exits

An exit has no routes and declares the run's result in `config.result`:
`name` uses the decision ID syntax and `outcome` is `success` or `failure`. A
flow may have several exits; exits sharing a name must agree on outcome.
Reaching an exit completes the run with that outcome, so a completed run with
outcome `failure` is not a success. Every non-exit node must have a path to an
exit, and a node without a route to follow never ends the run successfully.

The bundled flows expose two results: `completed` (`success`) from their done
exit and `blocked` (`failure`) from `failed.md`. Context values never override
the reached exit's result.

## Roles

A role is `roles/<name>.md` in the project's definitions or the bundled library, with
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
may set its launch limit under `budget` (`concurrent_runs`, default 4).
`budget` without `standing: true` fails
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

An `agent_task` node runs its agent in the run's working directory unless it
sets `runtime.workdir_from: <context key>`. The agent then runs in the
directory that key names, under the same rules as a subflow's
`manager.child_workdir_from` (see Subflows); a value that does not resolve
fails the node before the agent starts. `workdir_from` is only allowed on
`agent_task` nodes.

```yaml
runtime:
  workdir_from: context.workspace.path
```

Nodes sharing a session should use the same directory: Claude Code keys its
stored sessions by working directory, so a session resumed from a different
directory starts fresh with a `continuity_reset` event.

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
  "spark_version": "<version>",
  "sessions": {"implementer": ["work"]}
}
```

`sessions` lists the nodes of each
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
  exit_codes:
    '0': done
  output_map:
    context.workspace.path: path
contracts:
  writes_context:
  - context.workspace.path
routes:
  done:
    to: implement
    description: The workspace is ready at the recorded path.
---

```sh
mkdir -p "${SPARK_PROJECT_WORK_DIR}/checkouts"
worktree="${SPARK_PROJECT_WORK_DIR}/checkouts/${SPARK_RUN_ID}"
git worktree add -b "spark/task/${SPARK_RUN_ID}" "$worktree" >/dev/null
printf '{"path":"%s"}' "$worktree"
```
````

## Tool Decisions

A tool node declares exactly one decision adapter in `config`:

- `exit_codes` maps exit codes (decimal integer keys) to declared decisions,
  as in `prepare_workspace.md` above. Every declared decision must be selected
  by some code. An unmapped exit code or signal is an execution error with
  the exit code and stderr as evidence.
- `decision_field` names a top-level field of the single JSON object the
  command prints on stdout; its string value is the decision. The command
  must exit 0: any nonzero exit, missing field, malformed output or undeclared
  value is an execution error.

A validation step that reports both results as decisions uses the JSON mode,
so only its wrapper's own failures are errors:

````markdown
---
type: tool
title: Validate
config:
  decision_field: decision
  env_map:
    SPARK_VALIDATION_COMMAND: context.task.validation_command
    SPARK_WORKSPACE_PATH: context.workspace.path
  output_map:
    context.validation.command_exit_code: command_exit_code
contracts:
  writes_context:
  - context.validation.command_exit_code
routes:
  passed:
    to: finalize_commit
    description: The configured repository validation command returned zero and its evidence was captured.
  rejected:
    to: diagnose_validation
    description: That command returned nonzero; diagnose the captured output before further implementation.
---

```sh
set -eu
cd "$SPARK_WORKSPACE_PATH"
status=0
sh -ec "$SPARK_VALIDATION_COMMAND" >/dev/null 2>&1 || status=$?
if [ "$status" -eq 0 ]; then decision=passed; else decision=rejected; fi
printf '{"decision":"%s","command_exit_code":%d}\n' "$decision" "$status"
```
````

Document the exit codes a script expects. Do not map every nonzero status to
a domain decision when that would hide setup errors: a `rejected` decision
should mean only that the check did not pass.

## Switches

A `switch` node routes on context data without running a command or an
agent. It reads `config.on` once, compares it with `config.cases` by exact
equality, and selects `config.otherwise` for any other present scalar:

```yaml
type: switch
title: Check Conflicts
config:
  cases:
    '0': clean
  on: context.integration.conflict_count
  otherwise: conflicts
contracts:
  reads_context:
  - context.integration.conflict_count
routes:
  clean:
    to: finish_merge
    description: Integration preparation completed and the staged merge has zero unresolved conflicts.
  conflicts:
    to: resolve_conflicts
    description: Integration preparation completed and the staged merge contains unresolved conflicts.
```

- Case keys are JSON scalars: `'0'` is the number 0, `'true'` the boolean
  true, and `'"ready"'` the string ready (the string "0" is `'"0"'`). There is
  no truthiness or type coercion; null, arrays, objects and duplicate values
  (such as `0` and `0.0`) are rejected.
- `otherwise` is required. A missing key or a non-scalar value is an
  execution error, never `otherwise`.
- Each case and `otherwise` names a declared decision, and every declared
  decision is used by one of them.
- `on` must be listed in `contracts.reads_context`, and some reachable
  predecessor must write it (`writes_context` or an `output_map` destination).
- A switch writes no context. Computations belong in a tool.

## Human Gates

A `human_gate` node's routes are its choices: each `label` is the displayed
text and the key is the answer's value. The body is the question.

```markdown
---
type: human_gate
title: Human Review
routes:
  approve:
    to: ship
    description: The proposal is approved for shipping.
    label: Approve
  request_fixes:
    to: revise
    description: The proposal needs revision; record the reviewer's feedback.
    label: Request Fixes
  stop:
    to: failed
    description: Stop this workflow without approving the proposal.
    label: Stop Run
---

Review the proposed change. Approve it, request fixes, or stop the run.
```

The gate waits until a person picks a declared choice; an answer that names
none is rejected and the gate keeps waiting. A timeout never approves. Route a
stop choice to a failure exit; external cancellation still aborts the run.

## Run Result Summaries

An exit node may opt in to producing the run's result:

```markdown
---
type: exit
title: Done
config:
  result:
    name: completed
    outcome: success
  result_summary: true
---
```

When the run reaches an opted-in exit — or fails, in which case the first
opted-in exit by node id counts — the runtime executes one summarizer agent
whose working directory is the run's artifact root. It reads the recorded transcripts
(`events.jsonl`, `checkpoint.json`, `logs/<node>/…`) directly and its markdown
response becomes the run result shown in the Runs view. The exit node's body,
when present, overrides the default instructions. A summarizer failure never
fails the run; the result falls back to the exit predecessor's response with
the error recorded.

## Subflows

A `subflow` node's `config.flow_ref` names another flow directory. It resolves
relative to the directory that contains the parent flow's directory, so
`software-development/implement-change` would refer to
`software-development/workers/child` as `workers/child`.
The resulting name resolves through the roots like any flow, so a project copy
of a bundled child replaces it.

A subflow's child run normally inherits the parent's working directory, or an
authored static `manager.child_workdir`. To bind the child to a directory
produced by an earlier node, use `manager.child_workdir_from` with a context
key. The runtime validates that the resolved directory exists and is either
inside the run's working directory or a linked git worktree of the same
repository. Declaring both `child_workdir` and `child_workdir_from` fails the
node. Run working directories are immutable once a run starts; to run a single
agent step in another workspace, use the agent_task's `runtime.workdir_from`
(see Sessions) instead of a child run.

The child starts from the parent's context, with `config.input_map`
(`child_key: parent_key`) copying parent values into child keys. To hand child
values back, `config.output_map` (`child_key: parent_key`) copies each listed
child key into the parent when the child run ends, whether it succeeded or
failed; a listed child key that is absent clears the parent key. Every
`parent_key` must be listed in the subflow node's `contracts.writes_context`.
`context.stack.child.*` reports the child's status as before.

`config.results` maps each terminal result name the child can reach to a
local decision, and `routes` declares exactly those decisions. Every child
result must be mapped and every local decision used; several results may map
to one decision. Validation checks the mapping against the resolved child.

```yaml
config:
  flow_ref: workers/child
  results:
    completed: child_completed
    blocked: child_blocked
  output_map:
    context.task.summary: context.implementation.summary
contracts:
  writes_context:
  - context.implementation.summary
routes:
  child_completed:
    to: review
    description: The child reached completed; review its artifacts.
  child_blocked:
    to: review
    description: The child reached blocked; audit its evidence before continuing.
```

A child that reaches a failure exit has still completed, with its named
result. A child that fails with an execution, routing or contract error fails
the parent with the child's reason; it never maps to a result.

`implement.md`:

```markdown
---
type: agent_task
title: Implement
role: author
runtime:
  workdir_from: context.workspace.path
contracts:
  response: decision_envelope
routes:
  ready_for_review:
    to: evaluate
    description: The implementation pass is complete and ready for independent review.
  unfinished:
    to: implement
    description: Required implementation work remains and can be continued in the same session.
  blocked:
    to: failed
    description: The task cannot be completed without unavailable input or authority; record the blocker in notes.
---

Implement the task in the current working directory. Choose `ready_for_review`
once the pass is committed. Respond with ONLY a JSON decision envelope.
```

## Parallel Branches

A `parallel` node forks into the branch entry nodes in `config.branches` and
names its paired `fan_in` in `config.join`. It has no routes.

```yaml
type: parallel
title: Review Design
config:
  branches:
  - codebase_check
  - critic
  - naive_reader
  join: fan_in
  max_parallel: 3
  tolerate_branch_execution_errors: true
```

- Each branch runs from its entry node under the same decision, error, retry
  and visit rules as the root. A branch ends when one of its routes reaches
  the paired join; the join itself runs once in the parent after every branch
  settles, and routes on with `next`.
- A branch may also end at a failure exit. Success exits inside a branch,
  routes that bypass the join, overlapping branch interiors and nested forks
  are rejected.
- A branch that reaches a failure exit stops the fork: running siblings are
  canceled, the join does not run, and the run completes with that exit's
  result.
- Any branch execution error fails the run by default. With
  `tolerate_branch_execution_errors: true`, provider, transport, spawn,
  timeout and exhausted-repair errors are recorded and the join still runs.
  Routing errors, `visit_limit_reached` and failure exits are never tolerated.

Branch contexts are never merged into the parent. Successors of the join read
`parallel.results`: one entry per declared branch, in declaration order, with
`id` (the branch entry node), `index`, `status` (`completed`, `error`,
`terminal_result` or `canceled`), that branch's own `context`, and an `error`
record for errors. Branches may therefore write the same key without
overwriting each other. A node that synthesizes branch work reads each
completed entry's `context` and reports missing or failed branches instead of
counting them as covered. Branches themselves start without `parallel.results`,
so an entry never contains an earlier fork's results.

## Loops and Visit Limits

A visit is one entry into a node through routing. Every node may be visited
32 times per run; `runtime.max_visits` sets its own limit. Counts cover the
whole run: crash recovery, retries and continues keep them. A child run counts
its own nodes, and each fork invocation counts its branch nodes from zero.

The runtime selects the decision's route, then checks its target's visits. A
target that has used all its visits fails the run with
`visit_limit_reached`, naming the node, decision, target, count and limit; the
runtime never picks a different route instead. Agents still see every
decision, with exhausted targets marked as ending the run, so a visit limit
never changes what an honest decision is.

A loop limited to three diagnosis passes fails on the fourth rejection:

```yaml
# validate.md
routes:
  passed:
    to: finalize_commit
    description: ...
  rejected:
    to: diagnose_validation
    description: ...
# diagnose_validation.md
runtime:
  max_visits: 3
```

To continue at a limit rather than fail, make the choice an explicit decision
in the prompt, for example a reviewer that chooses between `rereview` and
`done`, and size `max_visits` so the honest decisions fit.

### Author and Isolated Reviewers

The bundled `software-development/design-change` flow combines parallel drafts,
sessions, isolated roles and a visit limit:

- `explore` forks four drafters, two aims across two models, each setting its
  model under `execution`. They write short approaches to
  `context.approach.draft` in their own branch contexts and change no files.
- `choose`, using the isolated `adversarial-reviewer` role, picks one draft
  against the request, recording why the others lost and any facts from them the
  design should use. `route_request` skips
  both steps when `context.request.artifact_path` names an existing design.
- `draft` and `resolve` use the `author` role and share `session: designer`,
  so the author keeps everything it learned while drafting.
- A `parallel` node, `review`, forks into reviewer branches that use the
  isolated `adversarial-reviewer` role. Each reviews through one lens and reads
  the document's path, never the author's reasoning. `critic`, `readiness` and
  `zoom_out` also read the request and constraints to judge fit, while
  `codebase_check` and `naive_reader` read only the document, which has to stand
  on its own. Each reviewer chooses `done` to the join
  and writes its complete findings to `context.review.lens_findings`, which
  lands in its own `parallel.results` entry. A response itself reaches later
  nodes only as `last_response`, cut to 200 characters.
- `review` tolerates branch execution errors, so a reviewer whose provider
  fails is recorded rather than failing the run.
- `synthesize`, after the `fan_in`, merges the reviewers' findings from
  `parallel.results` into one list without judging them again, and reports a
  branch that did not complete as a Blocker under its lens.
- `resolve` fixes each finding, contests it with checkable evidence, or asks
  the owner, then chooses `rereview` (back to `review`) or `done`. `review`
  sets `max_visits: 3`. Once `rereview` is marked out of visits, `resolve`
  chooses `done` and lists the unreviewed changes under Open questions;
  choosing `rereview` anyway fails the run with `visit_limit_reached`.

The approach is adapted from the elephant/goldfish model
([google/elephant-goldfish-model](https://github.com/google/elephant-goldfish-model),
Apache-2.0): an author who holds the context, reviewers who judge the
document without the author's reasoning, findings settled by fix, contest or escalation, and a bounded
re-review.

## Validation

Validate a flow directory before launching:

```bash
spark flow validate --dir crates/spark-assets/assets/flows/examples/simple-linear --text
```

Validation assembles the directory into a `FlowDefinition`, applies semantic
rules, and then verifies the runtime graph derived from the typed model. It
checks decision IDs and descriptions, route targets, each kind's decision
domain, exit results, tool, switch and subflow mappings, fork structure,
reachability from start and a path from every node to an exit. Removed
settings — `condition` and `weight` on routes, `conditional` nodes,
`runtime.error_policy`, `allow_partial`, `auto_status`, `goal_gate`,
`retry_target`, `fallback_retry_target`, `join_policy`, `join_k`,
`join_quorum` and human gate `config.decisions` — are rejected, not ignored,
and `context.workflow_outcome` has no effect on how a run ends.
A file inside `<project>/.spark/definitions` resolves roles through that
project's definitions, then the bundled library. `spark flow list|describe|get|validate`
take `--project <path>` to include a project's flows.
