# Spark Operations Guide

This guide is the packaged control-surface reference for agents operating Spark through its CLI and HTTP API.

Use it for launch, inspection, human-gate, and trigger tasks. OpenAPI remains the exhaustive schema source:

- Attractor: `/attractor/docs`, `/attractor/openapi.json`
- Workspace: `/workspace/docs`, `/workspace/openapi.json`

## Environment And Bootstrap

Installed or stable Spark instance:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

Source checkout workflow:

```bash
SPARK_HOME=~/.spark-dev uv run spark-server init
SPARK_HOME=~/.spark-dev uv run spark-server serve --reload --port 8010
SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow list
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
spark flow describe --flow simple-linear.dot --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/simple-linear.dot?surface=agent
```

Fetch raw DOT for a stored flow:

```bash
spark flow get --flow simple-linear.dot --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/simple-linear.dot/raw?surface=agent
```

Validate a file you are editing directly:

```bash
spark flow validate --file /absolute/path/to/flow.dot --text
```

Validate a stored flow through the server:

```bash
spark flow validate --flow simple-linear.dot --text
```

```bash
curl http://127.0.0.1:8000/workspace/api/flows/simple-linear.dot/validate
```

## Launch Runs And Create Run Requests

Launch a flow immediately against an explicit project:

```bash
spark run launch \
  --flow simple-linear.dot \
  --summary "Inspect the repo and summarize next steps." \
  --project /absolute/path/to/project
```

```bash
curl -X POST http://127.0.0.1:8000/workspace/api/runs/launch \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "simple-linear.dot",
    "summary": "Inspect the repo and summarize next steps.",
    "project_path": "/absolute/path/to/project"
  }'
```

Launch with explicit goal text or launch context:

```bash
spark run launch \
  --flow implement-review-loop.dot \
  --summary "Implement the approved change." \
  --project /absolute/path/to/project \
  --goal "Add the requested endpoint and tests." \
  --launch-context-json '{"context.request.ticket":"ABC-123"}'
```

Create a pending run request inside a conversation:

```bash
spark convo run-request \
  --conversation amber-otter \
  --flow spec-implementation/implement-spec.dot \
  --summary "Draft the implementation flow for the approved spec."
```

```bash
curl -X POST \
  http://127.0.0.1:8000/workspace/api/conversations/by-handle/amber-otter/flow-run-requests \
  -H 'Content-Type: application/json' \
  -d '{
    "flow_name": "spec-implementation/implement-spec.dot",
    "summary": "Draft the implementation flow for the approved spec."
  }'
```

Notes:

- `spark run launch` requires either `--conversation` or `--project`.
- `--goal-file` and `--launch-context-file` are available when inline text is inconvenient.
- Use `model` only when you need a launch-time override; otherwise let the flow defaults apply.

## Inspect Runs

Spark does not currently ship a dedicated agent CLI for run inspection. Use the UI or the HTTP API.

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
curl -N 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/events?after_sequence=1200'
```

Operational rules:

- `GET /attractor/pipelines/{id}` is the authoritative run-detail surface.
- `GET /attractor/pipelines/{id}/journal` is the primary durable history-read surface.
- `GET /attractor/pipelines/{id}/events` is live tail plus optional gap fill, not the primary history model.

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

Use the exact option value exposed by the question payload.

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
