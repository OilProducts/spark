# Spark Operations Guide

This guide is the packaged control-surface reference for agents operating Spark through its CLI and HTTP API.

Use it for launch, inspection, human-gate, trigger, and mission tasks. OpenAPI remains the exhaustive schema source:

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
- `spark flow validate --file ...` is local and does not require a running server.
- `SPARK_UI_DIR` overrides the served built UI directory when needed.

## Discover And Validate Flows

List agent-requestable flows:

```bash
spark flow list --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows?surface=agent
```

Describe one flow:

```bash
spark flow describe --flow examples/simple-linear.yaml --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear.yaml?surface=agent
```

Fetch raw YAML for a stored flow:

```bash
spark flow get --flow examples/simple-linear.yaml --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear.yaml/raw?surface=agent
```

Validate a file you are editing directly:

```bash
spark flow validate --file /absolute/path/to/flow.yaml --text
```

Validate a stored flow through the server:

```bash
spark flow validate --flow examples/simple-linear.yaml --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/examples/simple-linear.yaml/validate
```

Format a file you are editing directly:

```bash
spark flow format --file /absolute/path/to/flow.yaml
spark flow format --file /absolute/path/to/flow.yaml --write
```

`spark flow format --file` is local like file validation. The `--write` form updates only the target YAML file.

## Launch Runs, Recover Runs, And Create Run Requests

Assistant-runtime default:

- Include `--conversation <handle>` for launches and run requests so the resulting artifact appears inline in the active chat.
- Use `--project` without `--conversation` only for intentionally detached, project-scoped launches with no inline chat artifact.

Launch a flow immediately inside the active conversation:

```bash
spark run launch \
  --conversation amber-otter \
  --flow examples/simple-linear.yaml \
  --summary "Inspect the repo and summarize next steps."
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/launch \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter",
    "flow_name": "examples/simple-linear.yaml",
    "summary": "Inspect the repo and summarize next steps."
  }'
```

Launch a detached flow immediately against an explicit project:

```bash
spark run launch \
  --flow examples/simple-linear.yaml \
  --summary "Inspect the repo and summarize next steps." \
  --project /absolute/path/to/project
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/launch \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "examples/simple-linear.yaml",
    "summary": "Inspect the repo and summarize next steps.",
    "project_path": "/absolute/path/to/project"
  }'
```

Launch with explicit goal text or launch context:

```bash
spark run launch \
  --flow examples/implement-review-loop.yaml \
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
  --flow examples/simple-linear.yaml \
  --summary "Inspect the repo and summarize next steps." \
  --project "$PWD" \
  --launch-context-json '{"context.request.summary":"Inspect the checkout."}'
```

Create a pending run request inside a conversation:

```bash
spark convo run-request \
  --conversation amber-otter \
  --flow software-development/spec-implementation/implement-spec.yaml \
  --summary "Draft the implementation flow for the approved spec."
```

```bash
curl -X POST \
  http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/flow-run-requests \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "software-development/spec-implementation/implement-spec.yaml",
    "summary": "Draft the implementation flow for the approved spec."
  }'
```

Notes:

- `spark run launch` requires either `--conversation` or `--project`.
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
  --flow examples/simple-linear.yaml \
  --conversation amber-otter
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/<run_id>/continue \
  -H 'Content-Type: application/json' \
  -d '{
    "conversation_handle": "amber-otter",
    "start_node": "run_milestone",
    "flow_source_mode": "flow_name",
    "flow_name": "examples/simple-linear.yaml"
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

List triggers:

```bash
spark trigger list --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/triggers
```

Describe one trigger:

```bash
spark trigger describe --id <trigger_id> --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/triggers/<trigger_id>
```

Create a trigger from JSON:

```bash
spark trigger create --json /absolute/path/to/trigger.json
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/triggers \
  -H 'Content-Type: application/json' \
  -d @/absolute/path/to/trigger.json
```

Patch a trigger:

```bash
spark trigger update --id <trigger_id> --json /absolute/path/to/trigger-patch.json
```

```bash
curl -X PATCH http://127.0.0.1:8000/workspace/api/triggers/<trigger_id> \
  -H 'Content-Type: application/json' \
  -d @/absolute/path/to/trigger-patch.json
```

Delete a trigger:

```bash
spark trigger delete --id <trigger_id>
```

```bash
curl -X DELETE http://127.0.0.1:8000/workspace/api/triggers/<trigger_id>
```

Trigger operation notes:

- Trigger definitions are project-scoped and persisted through the Workspace trigger API.
- Protected triggers reject edits and deletes through the same CLI and HTTP surfaces.
- Webhook triggers use the shared Workspace webhook ingress route and validate their configured secret before dispatch.
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

- Packaged installs use `~/.spark` unless `SPARK_HOME` or `--data-dir` points elsewhere.
- The service loads provider secrets from `$SPARK_HOME/config/provider.env` when that file exists.
- Source checkouts should use `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init` and `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --port 8010` instead of mutating a stable packaged runtime.
- Packaged container workflows use `compose.package.yaml` and keep host-visible runtime state under the configured Spark home volume.

## Project missions

Missions are explicitly managed project records of an intended outcome. A mission that has not started behaves like a plain board card: inspect the queue before work and record useful issue detail in the optional plain-text description (the objective); no template or mandatory sections are needed. Do not infer or import missions from change requests automatically.

```sh
spark mission list --project /absolute/project
spark mission get --project /absolute/project --id mission-ID
spark mission create --project /absolute/project --json mission.json
spark mission update --project /absolute/project --id mission-ID --json - < update.json
spark mission start --project /absolute/project --id mission-ID
spark mission send --project /absolute/project --id mission-ID --message "Prefer the smaller fix"
spark mission events --project /absolute/project --id mission-ID --after 0
```

`--json` reads a file or `-` for stdin; output is JSON. Create requires only a title:

```json
{"fields":{"title":"Deliver search","description":"Search should return matching documents."},"actor":"assistant"}
```

Updates require the revision from get/list, with only changed fields:

```json
{"revision":1,"fields":{"stage":"planning","description":"Decide which document types are in scope."},"note":"Recorded the scope question","actor":"assistant"}
```

Stages are `backlog` (default), `planning`, `ready`, `in_progress`, `review`, and `done`. All are manually editable, including Done without a note. Reopen by selecting an earlier stage. Archive with `fields.archived: true` and restore with `false`. Optional activity notes and human/assistant attribution are retained. On a revision conflict, reread and reconcile; never blindly retry stale edits. Listing returns `missions`, ordered by creation time then ID. Moving a card launches nothing; ordinary conversation run requests still require approval.

Starting a mission is an explicit action: `spark mission start` moves it to In progress and posts `mission.started` to its inbox. From then on the mission owns runs and reacts to their events (`run.completed`, `run.failed`, `run.canceled`, `run.waiting`, `run.signal`, and `human.message`). `spark mission send` posts a `human.message`, which also clears an `attention` state and prompts the next reaction. Events are processed in order; `execution.substate` reports `idle`, `running`, `reasoning`, `waiting` (a human gate or an exhausted budget, named in `execution.reason`), or `attention` (a failed reaction or launch, or a reaction or hook closing the mission as failed or canceled). Attention holds until a human sends a message or resumes, which then applies any actions it deferred; a budget wait never replaces it. A mission a human cancels or closes shows `idle` with the close reason.

Hooks (`fields.hooks`) react mechanically, first match wins, and anything unmatched goes to a reaction run of `fields.reaction_flow` (default `missions/react.yaml`):

```json
{"revision":3,"fields":{"hooks":[{"on":"run.completed","label":"build","status":"completed","do":{"launch":{"flow_name":"software-development/review-change.yaml","label":"review","context":{"context.request.source_ref":"HEAD"}}}},{"on":"run.signal","do":"ignore"}],"budget":{"concurrent_runs":4,"total_runs":25,"reactions":10}}}
```

Hook actions are `launch`, `close` (`{"status":"done|failed|canceled","reason":"..."}`), `ignore`, and `reason`. A reaction receives `context.mission` (objective, state, runs, and the pending event batch) and ends by writing `context.mission.directive` = `{"actions":[...]}` using `launch`, `set_state` (`{"markdown":"..."}`), and `close`; only reactions write the mission `state`. Closing as done moves the card to Review for a human to finish. Runs launched by a mission carry `context.spark_mission` and may report mid-flight by writing a JSON value to `context.mission.signal`. Pause, resume, cancel, and close are available at `POST /workspace/api/missions/{id}/pause|resume|cancel|close?project_path=...`.
