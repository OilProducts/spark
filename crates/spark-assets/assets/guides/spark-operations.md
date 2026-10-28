# Spark Operations Guide

This guide is the packaged control-surface reference for agents operating Spark through its CLI and HTTP API.

Use it for launch, inspection, human-gate, trigger, mission, and standing-role tasks. OpenAPI remains the exhaustive schema source:

- Attractor: `/attractor/docs`, `/attractor/openapi.json`
- Workspace: `/workspace/docs`, `/workspace/openapi.json`

Control-surface contract:

- Inside the assistant runtime, the canonical Spark agent control surface is the bare `spark ...` CLI.
- In a human source-checkout shell, run the same CLI commands with `cargo run -p spark-cli --bin spark -- ...` before installing the binaries.
- The examples below use the assistant-runtime form unless a source-checkout example is called out explicitly.

## Environment And Bootstrap

Installed or stable Spark instance:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

Source checkout workflow:

```bash
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --port 8010
SPARK_API_BASE_URL=http://127.0.0.1:8010 cargo run -p spark-cli --bin spark -- flow list
```

Operational rules:

- From a source checkout, always set `SPARK_HOME` before `spark-server init` or `spark-server serve`.
- From a source checkout, always set `SPARK_API_BASE_URL` before `spark ...` commands that talk to a running server.
- `spark flow validate --dir ...` is local and does not require a running server.
- `SPARK_UI_DIR` overrides the served built UI directory when needed.

## Discover And Validate Flows

Flows resolve project first, then bundled: a project's org repo
(`<project>/.spark/agent-org/flows`) replaces any bundled flow of the same name
in `$SPARK_HOME/bundled/flows`. Pass `--project <path>` (or `project_path=` on
the HTTP routes) to include a project's flows; without it you see the bundled
library only. Every flow can be launched.

List flows:

```bash
spark flow list --project /absolute/path/to/project --text
```

```bash
curl 'http://127.0.0.1:8000/workspace/api/flows?project_path=/absolute/path/to/project'
```

Describe one flow:

```bash
spark flow describe --flow examples/simple-linear --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear
```

Print a stored flow's files (its layer and directory, then each file under a
`==> <file> <==` header; `--json` prints the JSON response):

```bash
spark flow get --flow examples/simple-linear
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear/files
```

Validate a flow directory you are editing directly:

```bash
spark flow validate --dir /absolute/path/to/flows/my-flow --text
```

Validate a stored flow through the server:

```bash
spark flow validate --flow examples/simple-linear --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear/validate
```

## Launch Runs, Recover Runs, And Create Run Requests

Assistant-runtime default:

- Include `--conversation <handle>` for launches and run requests so the resulting artifact appears inline in the active chat.
- Use `--project` without `--conversation` only for intentionally detached, project-scoped launches with no inline chat artifact.

Launch a flow immediately inside the active conversation:

```bash
spark run launch \
  --conversation amber-otter \
  --flow examples/simple-linear \
  --summary "Inspect the repo and summarize next steps."
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/launch \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter",
    "flow_name": "examples/simple-linear",
    "summary": "Inspect the repo and summarize next steps."
  }'
```

Launch a detached flow immediately against an explicit project:

```bash
spark run launch \
  --flow examples/simple-linear \
  --summary "Inspect the repo and summarize next steps." \
  --project /absolute/path/to/project
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/launch \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "examples/simple-linear",
    "summary": "Inspect the repo and summarize next steps.",
    "project_path": "/absolute/path/to/project"
  }'
```

Launch with explicit goal text or launch context:

```bash
spark run launch \
  --flow examples/implement-review-loop \
  --summary "Implement the approved change." \
  --project /absolute/path/to/project \
  --goal "Add the requested endpoint and tests." \
  --launch-context-json '{"context.request.ticket":"ABC-123"}' \
  --model gpt-5 \
  --llm-provider openai \
  --reasoning-effort high \
  --execution-profile native
```

Source-checkout launch against the development backend:

```bash
SPARK_API_BASE_URL=http://127.0.0.1:8010 cargo run -p spark-cli --bin spark -- run launch \
  --flow examples/simple-linear \
  --summary "Inspect the repo and summarize next steps." \
  --project "$PWD" \
  --launch-context-json '{"context.request.summary":"Inspect the checkout."}'
```

Launch a run from a conversation (agents use this; it launches immediately, prints the run id, and shows the launch in the conversation):

```bash
spark convo run-request \
  --conversation amber-otter \
  --flow software-development/spec-implementation/implement-spec \
  --summary "Draft the implementation flow for the approved spec."
```

```bash
curl -X POST \
  http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/flow-run-requests \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "software-development/spec-implementation/implement-spec",
    "summary": "Draft the implementation flow for the approved spec."
  }'
```

Notes:

- `spark run launch` requires either `--conversation` or `--project`.
- `spark convo run-request` launches any flow, resolved through the conversation's project.
- Before launching, read the flow's `inputs`, declared in the frontmatter of its `flow.md`, with `spark flow get --flow <name>`, which prints the flow's files. Supply required inputs using `--launch-context-json '<JSON object>'` or `--launch-context-file <path>`, keyed by the declared input keys (for example, `context.request.source_ref`). `--summary` describes the request; it does not populate workflow inputs.
- `--goal-file` and `--launch-context-file` are available when inline text is inconvenient.
- Use `--model`, `--llm-provider`, `--reasoning-effort`, and `--execution-profile` only when you need launch-time overrides; otherwise let the flow defaults and project settings apply.
- `spark run launch` and `spark convo run-request` do not take `--llm-profile`; use flow defaults such as `ui_default_llm_profile`, or use `--llm-profile` on supported recovery commands.
- A launch created without `--conversation` is project-scoped only, so no flow-launch card appears in chat.

## Recover Runs

Use the workspace recovery commands instead of calling Attractor recovery endpoints directly. They preserve conversation and project scoping and return stable JSON payloads for agents.

Retry executes a fresh attempt under the same run ID with a restored automatic-retry
allowance. Retrying a parent recovers its linked failed children and nested
descendants in place; successful children are consumed without rerunning. Existing
working directories, snapshots, lineage, and prior attempt artifacts are preserved.
`started` reports execution acceptance, not success. Continue creates a new run.

Retry a run inside the active conversation:

```bash
spark run retry \
  --run <run_id> \
  --conversation amber-otter
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/<run_id>/retry \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter"
  }'
```

Continue a run from a checkpoint using the source run snapshot:

```bash
spark run continue \
  --run <run_id> \
  --start-node run_milestone \
  --flow-source-mode snapshot \
  --conversation amber-otter
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/<run_id>/continue \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter",
    "start_node": "run_milestone",
    "flow_source_mode": "snapshot"
  }'
```

Continue a run against a named flow definition:

```bash
spark run continue \
  --run <run_id> \
  --start-node run_milestone \
  --flow-source-mode flow_name \
  --flow examples/simple-linear \
  --conversation amber-otter
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/<run_id>/continue \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter",
    "start_node": "run_milestone",
    "flow_source_mode": "flow_name",
    "flow_name": "examples/simple-linear"
  }'
```

Continue with launch-time model overrides:

```bash
spark run continue \
  --run <run_id> \
  --start-node run_milestone \
  --flow-source-mode snapshot \
  --conversation amber-otter \
  --model gpt-5.4 \
  --llm-provider openai \
  --llm-profile default \
  --reasoning-effort high
```

Notes:

- `spark run retry` posts to `POST /workspace/api/runs/{run_id}/retry`.
- `spark run continue` posts to `POST /workspace/api/runs/{run_id}/continue`.
- When `--conversation` is supplied, Spark resolves the handle, validates that any explicit `--project` matches the conversation project, creates a conversation-visible `run_recovery` artifact, and publishes the updated conversation snapshot.
- The recovery artifact records the operation, source run, result run, status, project path, conversation context, continuation node and flow source, optional model/provider/profile/reasoning overrides, and any recovery error.
- Retry records the same source and result run id. Continue records the original run as `source_run_id` and the newly created run as `result_run_id`.
- For detached continue, omit `--project` to let Attractor inherit the source run working directory. Include `--project /absolute/path/to/project` only when intentionally overriding that working directory.
- For conversation-scoped continue, omit `--project` unless validating an explicit project path against the conversation.
- `--flow` is only sent when `--flow-source-mode flow_name`; snapshot continuations ignore it.

## Inspect Runs

Use the CLI for live run tailing and the Attractor HTTP API for authoritative durable queries.

Authoritative selected-run detail:

```bash
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>
```

Durable journal history, newest first:

```bash
curl 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/journal?limit=50'
```

Load older journal history:

```bash
curl 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/journal?limit=50&before_sequence=1200'
```

Live-tail events after an already loaded sequence:

```bash
spark run events <pipeline_id> --after 1200
spark run events <pipeline_id> --after 1200 --json
```

Operational rules:

- `GET /attractor/pipelines/{id}` is the authoritative run-detail surface.
- `GET /attractor/pipelines/{id}/journal` is the primary durable history-read surface.
- `GET /workspace/api/live/events` is the browser/operator live delivery stream. It multiplexes observed resources: conversations use `conversation_id` with required `conversation_project_path`, runs overview uses optional active-project `runs_project_path` and omits it for all-project runs, triggers use `triggers_project_path`, and selected-run tailing uses `run_id` only after selected-run detail plus durable journal hydration establish a cursor. `run_sequence` and `conversation_revision` are reconnect/catch-up cursors, not steady-state stream identity.
- `GET /attractor/pipelines/{id}/events` is deprecated for Spark UI/operator clients. Prefer `spark run events` or `GET /workspace/api/live/events?run_id=<pipeline_id>&run_sequence=1200`.

Other selected-run inspection surfaces:

```bash
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/checkpoint
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/context
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/questions
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/artifacts
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/graph-preview
```

Fetch one artifact inline or as a download:

```bash
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/artifacts/path/to/file.txt
curl -OJ 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/artifacts/path/to/file.txt?download=true'
```

## Answer Pending Human Gates

List pending questions:

```bash
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/questions
```

Submit an answer:

```bash
curl -X POST \
  http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/questions/<question_id>/answer \
  -H 'Content-Type: application/json' \
  -d '{"selected_value":"approve"}'
```

Use the exact option value exposed by the question payload. `selected_value`
picks the route; it is not a feedback channel — text that matches no option
fails the gate as skipped.

To send feedback with the selection, add an optional `note`:

```bash
curl -X POST \
  http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/questions/<question_id>/answer \
  -H 'Content-Type: application/json' \
  -d '{"selected_value":"Request Fixes","note":"Tighten the error handling in the retry path."}'
```

The note reaches downstream nodes as the `human.gate.note` context value;
nodes that should see it declare it in `contracts.reads_context`. Each
answered gate overwrites `human.gate.note` (empty when no note was given).

## Manage Triggers

A trigger belongs to a project: its definition is `triggers/<id>.toml` in the project's org repo (`<project>/.spark/agent-org`), and a trigger is identified by its project plus its id.

List triggers, across registered projects or in one:

```bash
spark trigger list [--project /absolute/path/to/project] --text
```

```bash
curl 'http://127.0.0.1:8000/workspace/api/triggers?project_path=/absolute/path/to/project'
```

Describe one trigger:

```bash
spark trigger describe --project /absolute/path/to/project --id <trigger_id> --text
```

```bash
curl 'http://127.0.0.1:8000/workspace/api/triggers/<trigger_id>?project_path=/absolute/path/to/project'
```

Create a trigger from JSON in a registered project:

```bash
spark trigger create --project /absolute/path/to/project --json /absolute/path/to/trigger.json [--message <reason>]
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/triggers \
  -H 'Content-Type: application/json' \
  -d @/absolute/path/to/trigger.json   # includes "project_path" and optional "message"
```

Patch a trigger:

```bash
spark trigger update --project /absolute/path/to/project --id <trigger_id> --json /absolute/path/to/trigger-patch.json [--message <reason>]
```

```bash
curl -X PATCH 'http://127.0.0.1:8000/workspace/api/triggers/<trigger_id>?project_path=/absolute/path/to/project' \
  -H 'Content-Type: application/json' \
  -d @/absolute/path/to/trigger-patch.json
```

Delete a trigger:

```bash
spark trigger delete --project /absolute/path/to/project --id <trigger_id> --expected-revision <revision> [--message <reason>]
```

```bash
curl -X DELETE 'http://127.0.0.1:8000/workspace/api/triggers/<trigger_id>?project_path=/absolute/path/to/project&expected_revision=<revision>'
```

Trigger operation notes:

- Every create, update and delete commits the trigger's file into the org repo, with the given message or "Create/Edit/Delete trigger <name>". A definition edited and committed by hand is picked up on the next tick.
- `enabled` is runtime state kept by Spark: switching it writes no definition and makes no commit.
- Protected triggers reject edits and deletes through the same CLI and HTTP surfaces.
- Webhook triggers use the shared Workspace webhook ingress route and validate their secret before dispatch. Their key and secret hash live only in Spark's runtime state, never in the org repo; a webhook definition copied from another project has no key until its secret is regenerated.
- An action's `mode` is `static` (launch `flow_name` in the trigger's project, the default), `mission` (deliver to `mission_id` in the trigger's project), or `role` (deliver to the standing role `role` in the trigger's project). `flow_event` triggers fire only for runs in their own project.
- Production trigger runtime state, dispatch provenance, and live trigger events are emitted by the Rust trigger runtime. The browser/operator stream observes trigger changes through `/workspace/api/live/events` with `triggers_project_path`.
- Trigger-launched repository mutation policy is compatibility behavior, not a new safety policy defined by this guide.

## Packaged, Service, And Container Operation

Foreground packaged runtime:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

Linux user service:

```bash
spark-server service install --host 0.0.0.0 --port 8000
spark-server service status
spark-server service remove
```

Operational rules:

- Every Spark process, desktop, server and CLI alike, uses one data directory: `$SPARK_HOME` when it is set, else `~/.spark`. `spark-server --data-dir` overrides it for that server. Agents receive the server's data directory as `SPARK_HOME`, and their homes are `$SPARK_HOME/runtime/<agent>`.
- The service loads provider secrets from `$SPARK_HOME/config/provider.env` when that file exists.
- Source checkouts should use `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init` and `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --port 8010` instead of mutating a stable packaged runtime.
- Packaged container workflows use `compose.package.yaml` and keep host-visible runtime state under the configured Spark home volume.

## Project missions

A mission is a project objective plus a long-lived agent conversation that owns the mission's runs. A mission that has not started is a draft: record useful detail in the optional plain-text description (the objective); no template or mandatory sections are needed. Do not infer or import missions from change requests automatically.

```sh
spark mission list --project /absolute/project
spark mission get --project /absolute/project --id mission-ID
spark mission create --project /absolute/project --json mission.json
spark mission create --conversation <handle> --json mission.json
spark mission update --project /absolute/project --id mission-ID --json - < update.json
spark mission start --project /absolute/project --id mission-ID
spark mission send --project /absolute/project --id mission-ID --message "Prefer the smaller fix"
spark mission close --project /absolute/project --id mission-ID --status done --reason "Shipped and verified"
```

`--json` reads a file or `-` for stdin; output is JSON. Create requires only a title:

```json
{"fields":{"title":"Deliver search","description":"Search should return matching documents."},"actor":"assistant"}
```

Updates require the revision from get/list, with only changed fields. Editable fields are `title`, `description`, `archived`, `budget` (`{"concurrent_runs":4,"total_runs":25}` by default), and `playbook` (see below; fixed once the mission starts):

```json
{"revision":1,"fields":{"description":"Decide which document types are in scope."},"note":"Recorded the scope question","actor":"assistant"}
```

Create with `--conversation <handle>` instead of `--project` to make a mission from a chat: it takes the chat's project and records the chat as `source_conversation_id`. A `--project` that names a different project is rejected.

Archive with `fields.archived: true` and restore with `false`. On a revision conflict, reread and reconcile; never blindly retry stale edits. Listing returns `missions`, ordered by creation time then ID; `GET /workspace/api/missions` without `project_path` lists every project's missions.

Starting a mission creates its conversation (`conversation_id`) and sends the objective as the first turn. From then on, the mission's inbox delivers `run.completed`, `run.failed`, `run.canceled`, `run.question`, `run.waiting` (recovery decisions), and `human.message` events to that conversation as new turns: one turn per batch, oldest first, at most one turn in flight. `spark mission send` posts a `human.message`. The mission agent launches flows with `spark convo run-request`; each launch joins the mission's `runs` roster and carries `context.spark_owner` (`{"kind": "mission", "id": ...}`), so its events come back to the mission. A launch over budget is refused with the limit named. The agent closes the mission with `spark mission close`; `POST /workspace/api/missions/{id}/cancel?project_path=...` cancels in-flight runs and closes it.

Questions from all descendants return to the mission with their owning run ID. List open questions with `spark run questions --run <id>`. Answer using `spark run answer --run <owning run id> --question <question id> --option <value>` or `--text <text>`. The mission agent answers every question itself when the objective, playbook, run results and logs, and the repository give it the information, stating its reasoning; only when that information is missing does it ask the user for exactly what is missing, end the turn, and relay their reply without guessing. A mission is Running during its question-handling turn and Needs you if it ends with an unanswered question. Answers from the Runs view also clear the roster wait.

### Playbooks

A playbook is named, reusable guidance for a kind of mission: `playbooks/<name>.md`, Markdown with YAML frontmatter (`title`, `description`) and free-form instructions in the body. Playbooks resolve like flows and roles: the project's org repo (`<project>/.spark/agent-org/playbooks/`) first, then the bundled library (`$SPARK_HOME/bundled/playbooks/`, rewritten on every start, with playbooks such as `bug-report`). A project copy replaces the bundled playbook of the same name; to change a playbook for a project, edit or add it in the org repo. A mission resolves its playbook through its own project. The mission agent follows a playbook as the broad shape of the work, decides each step from run results and logs, and departs from it when the evidence calls for it.

```sh
spark playbook list --project /absolute/project
spark playbook get --project /absolute/project --name bug-report
```

To run a playbook against an issue, list the playbooks, create a mission with the playbook and the issue as its objective, and start it:

```json
{"fields":{"title":"Fix dropped parser input","description":"<the issue>","playbook":"bug-report"},"actor":"assistant"}
```

Create rejects an unknown playbook. On start, the mission snapshots the playbook into its record (`playbook`: name, title, description, and text), so later edits to the file do not change it, and the agent's pinned instructions include the playbook text after the objective.

`status` is derived on read: `draft` (not started), `running` (a turn or run is in flight or events are pending, and no run waits on a human gate), `needs_you` (a run waits on a human gate, or nothing is in flight or pending), or `closed`.

## Standing roles

A standing role is a role file (`roles/<name>.md`, resolved from the project's org repo, then bundled) bound to one long-lived conversation per project, `role-<project id>-<name>`. Two are bundled: the `coordinator`, the agent the user talks to, which exists in every project and is saved on first use, and the `chief-of-staff`, which owns and approves the org repo and starts only when asked. Each role keeps its memory in `<org>/memory/<name>/` (`index.md`, `log.md`), versioned in the org repo.

```bash
spark role list --project /absolute/project
spark role start --project /absolute/project --role chief-of-staff
spark role stop --project /absolute/project --role chief-of-staff
spark role send --project /absolute/project --role coordinator --conversation <sender handle> --message "Fixed the review flow"
```

Only a role whose file declares `standing: true` can be started; the bundled `coordinator` and `chief-of-staff` do. The Coordinator is standing in every project whatever its file says. The role file also sets the role's launch limits:

```yaml
standing: true
budget:
  concurrent_runs: 4
  runs_per_day: 20
```

`budget` and each field are optional and default to the values shown; `budget` without `standing: true` is invalid. The budget is read from the role file as the project resolves it, so a project raises or lowers it by editing the role's file in its org repo. Spark's role record keeps only runtime state: inbox, runs, recent launches, and whether the role is stopped.

The same operations are `GET /workspace/api/roles?project_path=...` and `POST /workspace/api/roles/{name}/start|stop|send?project_path=...` (`send` takes `{"conversation_handle", "message"}`).

A role wakes on a new turn when the user writes in its conversation, when a run launched from it reports back (launches carry `context.spark_owner` with `kind` `role`), when a `role`-mode trigger fires, when a mission created from its conversation closes, and when another conversation sends it a message; a message from another role names that role. Launches count against the role file's `budget`; a refused launch names the limit it hit. A stopped role still records events but delivers none until it starts again; its triggers stay enabled, and a `role` trigger skips runs the role launched itself and runs of other projects. A role file's `execution` settings apply to its conversation below the conversation's own model settings and above the project's.
