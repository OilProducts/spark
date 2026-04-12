# Spark

Spark is a workspace workbench for AI-assisted software delivery. It combines a FastAPI backend and React UI for registering local projects, authoring shared DOT workflows, running them against a selected project context, and coordinating project conversations, flow requests, and run launches.

## What Spark Does

- Register local project directories and persist project-scoped conversation and review state
- Author shared workspace workflows as DOT graphs, either visually or in raw DOT
- Parse, canonicalize, validate, and save flows through the backend
- Run project-aware pipelines with built-in handlers such as `codergen`, `tool`, `conditional`, `parallel`, `parallel.fan_in`, `wait.human`, and `stack.manager_loop`
- Stream live run events, inspect checkpoints and context, browse artifacts, and answer human-gate questions
- Work inside project-scoped AI conversation threads that can request or directly launch project-scoped workflow runs
- Review and approve flow run requests, or launch a run directly from the conversation surface

## Main User Workflow

1. Register or select a local project in Home.
2. Open or resume a project conversation thread.
3. Ask Spark to inspect the project, answer questions, or prepare a flow run request.
4. Review and approve the flow run request, or launch a flow directly from the conversation.
5. Monitor execution in the Execution and Runs views.

The UI also supports a direct authoring workflow: Home -> Editor -> Execution -> Runs.
Flow authoring is workspace-global rather than project-owned: you can open the Editor without selecting a project, while the Execution view keeps run-start actions disabled until a project context is selected. Trigger automation is also workspace-global and lives in its own top-level Triggers tab rather than inside project settings.

For flow authoring, use this progression:

- Hands-on tutorial: [docs/first-flow-tutorial.md](/Users/chris/projects/spark/docs/first-flow-tutorial.md)
- Product overview and authoring heuristics: this README
- Full raw DOT reference: [src/spark/guides/dot-authoring.md](/Users/chris/projects/spark/src/spark/guides/dot-authoring.md)

After direct flow edits, validate with `spark flow validate --file /path/to/flow.dot --text`.

## Flow Building Guide

Start with the smallest flow that matches the job:

- [src/spark/starter_flows/simple-linear.dot](/Users/chris/projects/spark/src/spark/starter_flows/simple-linear.dot): one pass through plan -> implement -> summarize
- [src/spark/starter_flows/implement-review-loop.dot](/Users/chris/projects/spark/src/spark/starter_flows/implement-review-loop.dot): plan -> implement -> review with an actual retry loop
- [src/spark/starter_flows/implement-from-plan.dot](/Users/chris/projects/spark/src/spark/starter_flows/implement-from-plan.dot): snapshot a plan file into `.spark/planflows/<run>/plan-source.md` plus `.spark/planflows/<run>/state.json`, expose that workspace via `context.planflow.*`, implement it, evaluate completion, and iterate
- [src/spark/starter_flows/spec-implementation/implement-spec.dot](/Users/chris/projects/spark/src/spark/starter_flows/spec-implementation/implement-spec.dot): greenfield spec-implementation program flow that keeps repo-local state under `.specflow/` and dispatches milestone workers
- [src/spark/starter_flows/human-review-loop.dot](/Users/chris/projects/spark/src/spark/starter_flows/human-review-loop.dot): explicit human approval or requested fixes
- [src/spark/starter_flows/parallel-review.dot](/Users/chris/projects/spark/src/spark/starter_flows/parallel-review.dot): fan-out / fan-in structure
- [src/spark/starter_flows/supervision/supervised-implementation.dot](/Users/chris/projects/spark/src/spark/starter_flows/supervision/supervised-implementation.dot): parent/child composition with `stack.manager_loop`

Use the flow `goal` as the user-facing stated goal for the run:

- In prompts and flow descriptions, write to the “stated goal”, not internal engine names like `graph.goal`.
- Direct runs from the editor currently use the flow's saved `goal`.
- Workspace chat run requests and `spark convo run-request --goal/--goal-file` can override that stated goal per run.

Use `launch_context` when one goal string is not enough:

- `launch_context` is first-class initial run state under `context.*`, not a prompt-only hack.
- Use it for structured launch details like request summaries, target paths, constraints, and acceptance criteria.
- Workspace run requests accept `launch_context`, and `spark convo run-request --launch-context-json/--launch-context-file` can populate it.
- Keep launch keys stable and semantic, for example `context.request.summary`, `context.request.target_paths`, and `context.request.acceptance_criteria`.
- In the flow editor, declare these inputs in Graph Settings -> Launch Inputs so direct execution can render a launch form from the flow itself.

Keep nodes single-purpose:

- A good node usually does one job: plan, implement, review, summarize, wait for human input, or supervise a child flow.
- Avoid combining planning, implementation, and review into one prompt if you want meaningful routing and retries.
- Prefer explicit labels on decision edges so the graph stays readable in the editor and run artifacts.

Pass context forward intentionally:

- Later nodes do not automatically understand why an earlier node was unhappy. If a stage should guide a later stage, it should emit `context_updates`.
- Use stable keys under `context.*` for cross-node feedback, for example `context.review.summary`, `context.review.required_changes`, and `context.review.blockers`.
- Author downstream prompts to consume that carryover directly. The implementation and planning starters in this repo do that on purpose.
- In the node inspector, use `Reads Context` and `Writes Context` to document that contract in the `.dot` itself instead of keeping it only in prompt prose.

Drive loops with outcome semantics, not prose:

- A node saying “needs fixes” in plain text does not create a retry loop by itself.
- Route retries off real outcomes such as `outcome=fail` and `outcome=success`.
- For Codex-backed review nodes, the most reliable pattern is to return a strict JSON status envelope so the backend can convert the model response into a real Attractor outcome plus `context_updates`.

Example review response shape:

```json
{
  "outcome": "fail",
  "notes": "Implementation is close but missing regression coverage.",
  "failure_reason": "Add tests before landing.",
  "context_updates": {
    "context.review.summary": "Behavior looks correct, but the change is not adequately covered by tests.",
    "context.review.required_changes": "Add a regression test for the changed path and rerun validation.",
    "context.review.blockers": ""
  }
}
```

Be explicit about routing behavior:

- If no conditioned edge matches, Attractor only considers unconditional edges next.
- A false conditioned edge is not a fallback route.
- If a non-failing node has no eligible next edge, the pipeline completes successfully.

Keep parent/child flows portable:

- Use relative `stack.child_dotfile` paths when bundling parent and child flows together.
- Avoid absolute `stack.child_workdir` values in shipped flows unless you really mean to force execution into a specific directory.
- Child flows should be reusable workers; parent flows should add supervision, governance, summary, or escalation rather than duplicating the child's work.

Use hooks and model defaults deliberately:

- A failing `tool.hooks.pre` prevents the tool command from running. Use it only when setup failure should block the tool.
- Use `tool.artifacts.paths`, `tool.artifacts.stdout`, and `tool.artifacts.stderr` when a tool node needs to preserve generated files or captured streams as run artifacts.
- `model_stylesheet` is best for broad model defaults; explicit node attrs still win over stylesheet matches.
- Graph defaults should establish a baseline. Node attrs should capture true per-stage exceptions.

## Architecture

- [src/attractor/](/Users/chris/projects/spark/src/attractor): Attractor runtime, pipeline engine, handlers, CLI, and mounted Attractor API
- [src/workspace/](/Users/chris/projects/spark/src/workspace): Spark workspace service, conversations, review artifacts, trigger subsystem, and mounted Workspace API
- [frontend/](/Users/chris/projects/spark/frontend): React 19 + Vite UI
- [src/spark/starter_flows/](/Users/chris/projects/spark/src/spark/starter_flows): curated starter `.dot` flows intended for first-run seeding
- [tests/fixtures/flows/](/Users/chris/projects/spark/tests/fixtures/flows): repo-only `.dot` fixtures used by tests and local development
- [tests/](/Users/chris/projects/spark/tests): backend tests, UI contracts, and acceptance assets
- [specs/](/Users/chris/projects/spark/specs): Attractor, workspace, frontend, and storage specifications

## Requirements

- Python 3.10+
- [`uv`](https://docs.astral.sh/uv/)
- Node.js 20+ and npm
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows
- `just` is optional, but the repo commands assume it when available

## Local Development

Prepare a fresh checkout:

```bash
just setup
```

Initialize the runtime tree and seed starter flows:

```bash
uv run spark-server init
```

`just setup` installs the Python dev environment with `uv sync --dev` and the frontend toolchain with `npm --prefix frontend ci`.

Install a stable wheel into `~/.spark/venv` and initialize the stable runtime:

```bash
just install
```

Run the full stack locally:

```bash
just run
```

This starts:

- the backend on `127.0.0.1:8010`
- the Vite frontend on `127.0.0.1:5173`

Open [http://127.0.0.1:5173](http://127.0.0.1:5173) for live frontend development.

The source-checkout dev wrapper intentionally uses a separate runtime home and port so it does not stomp on a stable installed Spark instance:

- `SPARK_HOME` defaults to `~/.spark-dev`
- backend port defaults to `8010`

Initialize that dev runtime explicitly with:

```bash
just init-dev
```

For Docker-based development:

```bash
just dev
```

That starts the backend on port `8000` and the frontend on port `5173` via `docker compose`.

## Backend-Only Usage

Start the server directly:

```bash
uv run spark-server serve --host 127.0.0.1 --port 8000
```

Useful development flags:

```bash
uv run spark-server serve \
  --host 127.0.0.1 \
  --port 8000 \
  --reload \
  --data-dir ~/.spark \
  --flows-dir ~/.spark/flows \
  --ui-dir ./frontend/dist
```

When a built UI is available, the backend serves it at [http://127.0.0.1:8000](http://127.0.0.1:8000).

## Runtime Data and Configuration

By default, Spark stores runtime data under `~/.spark`:

- `config/`
- `runtime/`
- `logs/`
- `workspace/projects/`
- `attractor/runs/`
- `flows/`

Important path overrides:

- `SPARK_HOME`
- `SPARK_FLOWS_DIR`
- `SPARK_UI_DIR`

`~/.spark/config/prompts.toml` stores user-configurable prompt templates and is created on first startup.

## API Overview

The canonical route inventory lives in [app.py](/Users/chris/projects/spark/src/spark_app/app.py), [server.py](/Users/chris/projects/spark/src/attractor/api/server.py), and [api.py](/Users/chris/projects/spark/src/workspace/api.py).

The root app is a mount host only. Canonical public API surfaces are:
- Attractor docs/OpenAPI under `/attractor/docs` and `/attractor/openapi.json`
- Workspace docs/OpenAPI under `/workspace/docs` and `/workspace/openapi.json`

Current API groups include:

- Attractor runtime and runs: `GET /attractor/status`, `GET /attractor/runs`
- Attractor pipeline execution: `POST /attractor/pipelines`, `GET /attractor/pipelines/{id}`, `POST /attractor/pipelines/{id}/cancel`
- Attractor pipeline inspection: `GET /attractor/pipelines/{id}/events`, `GET /attractor/pipelines/{id}/checkpoint`, `GET /attractor/pipelines/{id}/context`, `GET /attractor/pipelines/{id}/graph`, `GET /attractor/pipelines/{id}/artifacts`
- Attractor human-gate actions: `GET /attractor/pipelines/{id}/questions`, `POST /attractor/pipelines/{id}/questions/{question_id}/answer`
- Attractor flow management: `GET /attractor/api/flows`, `POST /attractor/api/flows`, `GET /attractor/api/flows/{name}`, `DELETE /attractor/api/flows/{name}`
- Workspace project management: `GET /workspace/api/projects`, `POST /workspace/api/projects/register`, `PATCH /workspace/api/projects/state`, `DELETE /workspace/api/projects`
- Workspace triggers and project metadata: `GET /workspace/api/triggers`, `POST /workspace/api/triggers`, `GET /workspace/api/triggers/{trigger_id}`, `PATCH /workspace/api/triggers/{trigger_id}`, `DELETE /workspace/api/triggers/{trigger_id}`, `POST /workspace/api/webhooks`, `GET /workspace/api/projects/metadata`, `POST /workspace/api/projects/pick-directory`
- Workspace conversations: `GET /workspace/api/projects/conversations`, `GET /workspace/api/conversations/{conversation_id}`, `GET /workspace/api/conversations/{conversation_id}/events`, `POST /workspace/api/conversations/{conversation_id}/turns`, `DELETE /workspace/api/conversations/{conversation_id}`
- Workspace run-launch workflows: `POST /workspace/api/conversations/by-handle/{conversation_handle}/flow-run-requests`, `POST /workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review`, `POST /workspace/api/runs/launch`

## Repository Commands

Useful `just` targets from [justfile](/Users/chris/projects/spark/justfile):

- `just clean`: remove generated build artifacts without deleting installed dependencies or runtime state
- `just setup`: install Python and frontend development dependencies for a fresh checkout
- `just run`: backend + Vite frontend for local development
- `just dev`: `docker compose up --build`
- `just test`: full Python test suite
- `just frontend-unit`: frontend unit tests
- `just ui-smoke`: Playwright smoke checks
- `just dot-lint`: DOT formatting lint regression
- `just build`: frontend build and wheel build

## Testing

Backend suite:

```bash
uv run pytest -q
```

Frontend unit tests:

```bash
npm --prefix frontend run test:unit
```

Frontend smoke tests:

```bash
just ui-smoke
```

`just ui-smoke` launches the product ASGI app from [`app.py`](/Users/chris/projects/spark/src/spark_app/app.py), waits for `GET /attractor/status`, and then runs `npm --prefix frontend run ui:smoke`.

## Packaging

Build the packaged UI and wheel:

```bash
just build
```

Or run the steps manually:

```bash
npm --prefix frontend run build
uv build
```

`uv build` stages the bundled UI into the package transiently from `frontend/dist` and cleans it afterward. The generated `src/spark_app/ui_dist` tree is not tracked in git.

Install the resulting wheel:

```bash
pip install dist/*.whl
```

## Notes

- Flow files are stored as canonical DOT and validated before save.
- Spark flow self-description lives in DOT via `spark.title` and `spark.description`, while workspace launch policy is stored separately in `~/.spark/config/flow-catalog.toml`.
- The agent-facing CLI exposes curated flow discovery commands with JSON default output: `spark flow list`, `spark flow describe --flow <name>`, and `spark flow get --flow <name>`.
- The editor supports both structured editing and raw DOT editing, including semantic-equivalence safety checks during handoff.
- The Runs view is intended for historical inspection, diagnostics, artifact browsing, and replaying execution context.
- Starter flow templates live in [src/spark/starter_flows/parallel-review.dot](/Users/chris/projects/spark/src/spark/starter_flows/parallel-review.dot), [src/spark/starter_flows/simple-linear.dot](/Users/chris/projects/spark/src/spark/starter_flows/simple-linear.dot), [src/spark/starter_flows/human-review-loop.dot](/Users/chris/projects/spark/src/spark/starter_flows/human-review-loop.dot), [src/spark/starter_flows/implement-from-plan.dot](/Users/chris/projects/spark/src/spark/starter_flows/implement-from-plan.dot), [src/spark/starter_flows/implement-review-loop.dot](/Users/chris/projects/spark/src/spark/starter_flows/implement-review-loop.dot), [src/spark/starter_flows/spec-implementation/implement-spec.dot](/Users/chris/projects/spark/src/spark/starter_flows/spec-implementation/implement-spec.dot), [src/spark/starter_flows/spec-implementation/implement-milestone.dot](/Users/chris/projects/spark/src/spark/starter_flows/spec-implementation/implement-milestone.dot), [src/spark/starter_flows/supervision/implementation-worker.dot](/Users/chris/projects/spark/src/spark/starter_flows/supervision/implementation-worker.dot), and [src/spark/starter_flows/supervision/supervised-implementation.dot](/Users/chris/projects/spark/src/spark/starter_flows/supervision/supervised-implementation.dot).
- Repo-only advanced/test fixtures live under [tests/fixtures/flows/](/Users/chris/projects/spark/tests/fixtures/flows).

## Project Status

Active development.
