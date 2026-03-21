# Spark Spawn

Spark Spawn is a project-scoped workflow workbench for AI-assisted software delivery. It combines a FastAPI backend and React UI for registering local projects, authoring DOT workflows, running them, and reviewing planning artifacts produced inside project conversations.

## What Spark Spawn Does

- Register local project directories and persist per-project workspace state
- Author workflows as DOT graphs, either visually or in raw DOT
- Parse, canonicalize, validate, and save flows through the backend
- Run project-aware pipelines with built-in handlers such as `codergen`, `tool`, `conditional`, `parallel`, `parallel.fan_in`, `wait.human`, and `stack.manager_loop`
- Stream live run events, inspect checkpoints and context, browse artifacts, and answer human-gate questions
- Work inside project-scoped AI conversation threads that can produce spec-edit proposals and execution cards
- Review and approve or reject spec edits, then review, revise, or approve execution plans for execution

## Main User Workflow

1. Register or select a local project in Home.
2. Open or resume a project conversation thread.
3. Ask Spark Spawn to help draft or refine a spec.
4. Review the resulting spec-edit proposal.
5. Approve the proposal to trigger execution planning.
6. Review the generated execution card.
7. Approve the execution card and launch a project-scoped workflow run.
8. Monitor execution in the Execution and Runs views.

The UI also supports a direct authoring workflow: Home -> Editor -> Execution -> Runs.

For agent-facing raw DOT editing guidance, see [src/sparkspawn/guides/dot-authoring.md](/Users/chris/tinker/sparkspawn/src/sparkspawn/guides/dot-authoring.md). After direct flow edits, validate with `sparkspawn-workspace validate-flow --flow <name> --text`.

## Flow Building Guide

Start with the smallest flow that matches the job:

- [starter-flows/simple-linear.dot](/Users/chris/tinker/sparkspawn/starter-flows/simple-linear.dot): one pass through plan -> implement -> summarize
- [starter-flows/implement-review-loop.dot](/Users/chris/tinker/sparkspawn/starter-flows/implement-review-loop.dot): plan -> implement -> review with an actual retry loop
- [starter-flows/human-review-loop.dot](/Users/chris/tinker/sparkspawn/starter-flows/human-review-loop.dot): explicit human approval or requested fixes
- [starter-flows/parallel-review.dot](/Users/chris/tinker/sparkspawn/starter-flows/parallel-review.dot): fan-out / fan-in structure
- [starter-flows/supervised-implementation.dot](/Users/chris/tinker/sparkspawn/starter-flows/supervised-implementation.dot): parent/child composition with `stack.manager_loop`

Use the flow `goal` as the user-facing stated goal for the run:

- In prompts and flow descriptions, write to the “stated goal”, not internal engine names like `graph.goal`.
- Direct runs from the editor currently use the flow's saved `goal`.
- Workspace chat flow-run requests and `sparkspawn-workspace flow-run --goal/--goal-file` can override that stated goal per run.

Use `launch_context` when one goal string is not enough:

- `launch_context` is first-class initial run state under `context.*`, not a prompt-only hack.
- Use it for structured launch details like request summaries, target paths, constraints, and acceptance criteria.
- Workspace flow-run requests accept `launch_context`, and `sparkspawn-workspace flow-run --launch-context-json/--launch-context-file` can populate it.
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

- [src/attractor/](/Users/chris/tinker/sparkspawn/src/attractor): Attractor runtime, pipeline engine, handlers, CLI, and mounted Attractor API
- [src/workspace/](/Users/chris/tinker/sparkspawn/src/workspace): Spark Spawn workspace service, conversations, review artifacts, trigger bindings, and mounted Workspace API
- [frontend/](/Users/chris/tinker/sparkspawn/frontend): React 19 + Vite UI
- [starter-flows/](/Users/chris/tinker/sparkspawn/starter-flows): curated starter `.dot` flows intended for first-run seeding
- [tests/fixtures/flows/](/Users/chris/tinker/sparkspawn/tests/fixtures/flows): repo-only `.dot` fixtures used by tests and local development
- [tests/](/Users/chris/tinker/sparkspawn/tests): backend tests, UI contracts, and acceptance assets
- [specs/](/Users/chris/tinker/sparkspawn/specs): Attractor, workspace, frontend, and storage specifications

## Requirements

- Python 3.10+
- [`uv`](https://docs.astral.sh/uv/)
- Node.js 20+ and npm
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows
- `just` is optional, but the repo commands assume it when available

## Local Development

Install dependencies:

```bash
uv sync --dev
npm --prefix frontend install
```

Initialize the runtime tree and seed starter flows:

```bash
uv run sparkspawn init
```

Run the full stack locally:

```bash
just run
```

This starts:

- the backend on `127.0.0.1:8000`
- the Vite frontend on `127.0.0.1:5173`

Open [http://127.0.0.1:5173](http://127.0.0.1:5173) for live frontend development.

For Docker-based development:

```bash
just dev
```

That starts the backend on port `8000` and the frontend on port `5173` via `docker compose`.

## Backend-Only Usage

Start the server directly:

```bash
uv run sparkspawn serve --host 127.0.0.1 --port 8000
```

Useful development flags:

```bash
uv run sparkspawn serve \
  --host 127.0.0.1 \
  --port 8000 \
  --reload \
  --data-dir ~/.sparkspawn \
  --flows-dir ~/.sparkspawn/flows \
  --ui-dir ./frontend/dist
```

When a built UI is available, the backend serves it at [http://127.0.0.1:8000](http://127.0.0.1:8000).

## Runtime Data and Configuration

By default, Spark Spawn stores runtime data under `~/.sparkspawn`:

- `config/`
- `runtime/`
- `logs/`
- `workspace/projects/`
- `attractor/runs/`
- `flows/`

Important path overrides:

- `SPARKSPAWN_HOME`
- `SPARKSPAWN_FLOWS_DIR`
- `SPARKSPAWN_UI_DIR`

`~/.sparkspawn/config/prompts.toml` stores user-configurable prompt templates and is created on first startup.

## API Overview

The canonical route inventory lives in [app.py](/Users/chris/tinker/sparkspawn/src/sparkspawn_app/app.py), [server.py](/Users/chris/tinker/sparkspawn/src/attractor/api/server.py), and [api.py](/Users/chris/tinker/sparkspawn/src/workspace/api.py).

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
- Workspace flow bindings and metadata: `GET /workspace/api/projects/flow-bindings`, `PUT /workspace/api/projects/flow-bindings/{trigger}`, `DELETE /workspace/api/projects/flow-bindings/{trigger}`, `GET /workspace/api/projects/metadata`, `POST /workspace/api/projects/pick-directory`
- Workspace conversations: `GET /workspace/api/projects/conversations`, `GET /workspace/api/conversations/{conversation_id}`, `GET /workspace/api/conversations/{conversation_id}/events`, `POST /workspace/api/conversations/{conversation_id}/turns`, `DELETE /workspace/api/conversations/{conversation_id}`
- Workspace review workflows: `POST /workspace/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/approve`, `POST /workspace/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/reject`, `POST /workspace/api/conversations/{conversation_id}/execution-cards/{execution_card_id}/review`

## Repository Commands

Useful `just` targets from [justfile](/Users/chris/tinker/sparkspawn/justfile):

- `just run`: backend + Vite frontend for local development
- `just dev`: `docker compose up --build`
- `just test`: full Python test suite
- `just frontend-unit`: frontend unit tests
- `just ui-smoke`: Playwright smoke checks
- `just dot-lint`: DOT formatting lint regression
- `just build`: frontend build, UI dist sync, and wheel build

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
npm --prefix frontend run ui:smoke
```

## Packaging

Build the packaged UI and wheel:

```bash
just build
```

Or run the steps manually:

```bash
npm --prefix frontend run build
./scripts/sync_ui_dist.sh
uv build
```

Install the resulting wheel:

```bash
pip install dist/*.whl
```

## Notes

- Flow files are stored as canonical DOT and validated before save.
- Spark Spawn flow self-description lives in DOT via `sparkspawn.title` and `sparkspawn.description`, while workspace launch policy is stored separately in `~/.sparkspawn/config/flow-catalog.toml`.
- The agent-facing workspace CLI exposes curated flow discovery commands with JSON default output: `sparkspawn-workspace list-flows`, `sparkspawn-workspace describe-flow --flow <name>`, and `sparkspawn-workspace get-flow --flow <name>`.
- The editor supports both structured editing and raw DOT editing, including semantic-equivalence safety checks during handoff.
- The Runs view is intended for historical inspection, diagnostics, artifact browsing, and replaying execution context.
- Starter flow templates live in [starter-flows/plan-generation.dot](/Users/chris/tinker/sparkspawn/starter-flows/plan-generation.dot), [starter-flows/parallel-review.dot](/Users/chris/tinker/sparkspawn/starter-flows/parallel-review.dot), [starter-flows/simple-linear.dot](/Users/chris/tinker/sparkspawn/starter-flows/simple-linear.dot), [starter-flows/human-review-loop.dot](/Users/chris/tinker/sparkspawn/starter-flows/human-review-loop.dot), [starter-flows/implement-review-loop.dot](/Users/chris/tinker/sparkspawn/starter-flows/implement-review-loop.dot), [starter-flows/implementation-worker.dot](/Users/chris/tinker/sparkspawn/starter-flows/implementation-worker.dot), and [starter-flows/supervised-implementation.dot](/Users/chris/tinker/sparkspawn/starter-flows/supervised-implementation.dot).
- Repo-only advanced/test fixtures live under [tests/fixtures/flows/](/Users/chris/tinker/sparkspawn/tests/fixtures/flows).

## Project Status

Active development.
