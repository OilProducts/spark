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

- Hands-on tutorial: [docs/first-flow-tutorial.md](docs/first-flow-tutorial.md)
- Product overview and authoring heuristics: this README
- Full raw DOT reference: [src/spark/guides/dot-authoring.md](src/spark/guides/dot-authoring.md)
- Task-oriented CLI/API operations guide: [src/spark/guides/spark-operations.md](src/spark/guides/spark-operations.md)

When working from a source checkout, validate direct flow edits with `uv run spark flow validate --file /path/to/flow.dot --text`.

## Flow Building Guide

Start with the smallest flow that matches the job:

Examples:

- [src/spark/flows/examples/simple-linear.dot](src/spark/flows/examples/simple-linear.dot): one pass through plan -> implement -> summarize
- [src/spark/flows/examples/implement-review-loop.dot](src/spark/flows/examples/implement-review-loop.dot): plan -> implement -> review with an actual retry loop
- [src/spark/flows/examples/human-review-loop.dot](src/spark/flows/examples/human-review-loop.dot): explicit human approval or requested fixes
- [src/spark/flows/examples/parallel-review.dot](src/spark/flows/examples/parallel-review.dot): fan-out / fan-in structure
- [src/spark/flows/examples/supervision/supervised-implementation.dot](src/spark/flows/examples/supervision/supervised-implementation.dot): parent/child composition with `stack.manager_loop`

Packaged workflows:

- [src/spark/flows/software-development/implement-change-request.dot](src/spark/flows/software-development/implement-change-request.dot): read a durable change request from `changes/<CR-id>/request.md`, keep runtime state under `.spark/change-requests/<CR-id>/`, implement it, evaluate completion, and write `changes/<CR-id>/result.md`
- [src/spark/flows/software-development/spec-implementation/implement-spec.dot](src/spark/flows/software-development/spec-implementation/implement-spec.dot): software-development program flow that keeps committed spec artifacts under `specs/<slug>/`, keeps runtime state under `.spark/spec-implementation/<slug>/`, and dispatches milestone workers from the stable `.spark/.../current/` alias

Use the flow `goal` as the user-facing stated goal for the run:

- In prompts and flow descriptions, write to the “stated goal”, not internal engine names like `graph.goal`.
- Direct runs from the editor currently use the flow's saved `goal`.
- Workspace chat run requests and `uv run spark convo run-request --goal/--goal-file` can override that stated goal per run.

Use `launch_context` when one goal string is not enough:

- `launch_context` is first-class initial run state under `context.*`, not a prompt-only hack.
- Use it for structured launch details like request summaries, target paths, constraints, and acceptance criteria.
- Workspace run requests accept `launch_context`, and `uv run spark convo run-request --launch-context-json/--launch-context-file` can populate it.
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

- [src/attractor/](src/attractor): Attractor runtime, pipeline engine, handlers, CLI, and mounted Attractor API
- [src/spark/workspace/](src/spark/workspace): Spark workspace service, conversations, review artifacts, trigger subsystem, and mounted Workspace API
- [src/spark/chat/](src/spark/chat): Spark chat orchestration, prompt templates, session plumbing, and chat response parsing
- [frontend/](frontend): React 19 + Vite UI
- [src/spark/flows/](src/spark/flows): packaged `.dot` flows shipped with Spark; examples live under [src/spark/flows/examples/](src/spark/flows/examples)
- [tests/fixtures/flows/](tests/fixtures/flows): repo-only `.dot` fixtures used by tests and local development
- [tests/](tests): backend tests, UI contracts, and acceptance assets
- [specs/](specs): Attractor, workspace, frontend, and storage specifications

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

Initialize the runtime tree and seed packaged flows:

```bash
uv run spark-server init
```

`just setup` installs the Python dev environment with `uv sync --dev` and the frontend toolchain with `npm --prefix frontend ci`.

Install a stable wheel into `~/.spark/venv` and initialize the stable runtime:

```bash
just install
```

Start the installed server in the foreground with:

```bash
~/.spark/venv/bin/spark-server serve --host 127.0.0.1 --port 8000
```

On Linux, install and start the background service explicitly with:

```bash
just install-systemd
```

The installed service binds every interface by default. To bind the service to loopback only, set `SPARK_HOST=127.0.0.1` when installing it:

```bash
SPARK_HOST=127.0.0.1 just install-systemd
```

You can also bypass `just` and install the user service directly with an explicit host and port:

```bash
~/.spark/venv/bin/spark-server service install --host 0.0.0.0 --port 8000 --data-dir ~/.spark
```

Use `~/.spark/venv/bin/spark-server service status` to inspect it or `~/.spark/venv/bin/spark-server service remove` to stop and unregister it.

If `just install-systemd` appears to pause after the wheel installation completes, it is likely waiting for `systemctl --user restart spark.service`. An existing server with open browser connections, especially event streams, may take up to systemd's default stop timeout to restart. Check the service with:

```bash
systemctl --user status spark.service --no-pager --full
```

### Provider API Keys

Unified provider-backed chat and workflow runs read provider API keys from the Spark server process environment:

- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `GEMINI_API_KEY` or `GOOGLE_API_KEY`
- `OPENROUTER_API_KEY`
- `LITELLM_BASE_URL` for a user-operated LiteLLM proxy; `LITELLM_API_KEY` is optional.

For the Linux user-level systemd service, keep keys outside the project repo and add them through a private environment file under `SPARK_HOME`. For a stable install, that file lives at `~/.spark/config/provider.env`:

```bash
mkdir -p ~/.spark/config
chmod 700 ~/.spark/config
$EDITOR ~/.spark/config/provider.env
chmod 600 ~/.spark/config/provider.env
```

Add the key values to `$SPARK_HOME/config/provider.env`:

```bash
ANTHROPIC_API_KEY=your_key_here
OPENROUTER_API_KEY=your_key_here
LITELLM_BASE_URL=https://litellm.example.com/v1
```

OpenRouter defaults to `https://openrouter.ai/api/v1` and accepts optional `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, and `OPENROUTER_TITLE`. OpenRouter and LiteLLM require explicit model selection in chat and workflow runs.

The generated user service now attaches that file automatically with an optional `EnvironmentFile=-$SPARK_HOME/config/provider.env` entry.

If you need an advanced override, you can still attach a drop-in manually:

```bash
mkdir -p ~/.config/systemd/user/spark.service.d
$EDITOR ~/.config/systemd/user/spark.service.d/override.conf
```

Use this override:

```ini
[Service]
EnvironmentFile=%h/.spark/config/provider.env
```

Reload and restart the service:

```bash
systemctl --user daemon-reload
systemctl --user restart spark.service
systemctl --user status spark.service --no-pager
```

Do not put API keys in flow DOT, launch context, project files, or checked-in configuration.

Run the full stack locally:

```bash
just dev-run
```

This starts:

- the backend on `127.0.0.1:8010`
- the Vite frontend on `127.0.0.1:5173`

Open [http://127.0.0.1:5173](http://127.0.0.1:5173) for live frontend development.

The source-checkout dev wrapper intentionally uses a separate runtime home and port so it does not stomp on a stable installed Spark instance:

- `SPARK_HOME` defaults to `~/.spark-dev`
- backend port defaults to `8010`
- provider secrets, when present, are read from `~/.spark-dev/config/provider.env` unless `SPARK_HOME` is set

Initialize that dev runtime explicitly with:

```bash
SPARK_HOME=~/.spark-dev uv run spark-server init
```

For Docker-based development:

```bash
just dev-docker
```

That starts the backend on port `8000` and the frontend on port `5173` via `docker compose`.
If `~/.spark-dev/config/provider.env` exists, the wrapper sources it before launching Docker.

The tracked `compose.yaml` is public-safe by default and does not mount personal Codex auth, config, or skills files from your machine.
It passes through provider environment variables including `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_ORG_ID`, `OPENAI_PROJECT_ID`, `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`, `GEMINI_API_KEY`, `GEMINI_BASE_URL`, `GOOGLE_API_KEY`, `OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, `OPENROUTER_TITLE`, `LITELLM_BASE_URL`, and `LITELLM_API_KEY`.
If you want containerized Codex auth or custom skills, add them in an untracked `compose.override.yaml`, for example:

```yaml
services:
  backend:
    volumes:
      - /path/to/auth.json:/codex-seed/auth.json:ro
      - /path/to/config.toml:/codex-seed/config.toml:ro
      - /path/to/skills:/codex-runtime/.codex/skills:ro
```

That container path matches the tracked `compose.yaml`, which explicitly sets `ATTRACTOR_CODEX_RUNTIME_ROOT=/codex-runtime` for Docker development.

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

Spark-managed Codex runtime state defaults to `~/.spark/runtime/codex`, or `<SPARK_HOME>/runtime/codex` when `SPARK_HOME` is set. Set `ATTRACTOR_CODEX_RUNTIME_ROOT` only when you need to force a different runtime root.

`~/.spark/config/prompts.toml` stores user-configurable prompt templates and is created on first startup.

## API Overview

For a task-oriented packaged reference that pairs Spark CLI commands with the matching HTTP routes, use [src/spark/guides/spark-operations.md](src/spark/guides/spark-operations.md).

The canonical route inventory lives in [app.py](src/spark/app.py), [server.py](src/attractor/api/server.py), and [api.py](src/spark/workspace/api.py).

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
- Workspace triggers and project metadata: `GET /workspace/api/triggers`, `POST /workspace/api/triggers`, `GET /workspace/api/triggers/{trigger_id}`, `PATCH /workspace/api/triggers/{trigger_id}`, `DELETE /workspace/api/triggers/{trigger_id}`, `POST /workspace/api/webhooks`, `GET /workspace/api/projects/metadata`, `GET /workspace/api/projects/browse`
- Workspace conversations: `GET /workspace/api/projects/conversations`, `GET /workspace/api/conversations/{conversation_id}`, `GET /workspace/api/conversations/{conversation_id}/events`, `POST /workspace/api/conversations/{conversation_id}/turns`, `DELETE /workspace/api/conversations/{conversation_id}`
- Workspace run-launch workflows: `POST /workspace/api/conversations/by-handle/{conversation_handle}/flow-run-requests`, `POST /workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review`, `POST /workspace/api/runs/launch`

## Repository Commands

Useful `just` targets from [justfile](justfile):

- `just setup`: install Python and frontend development dependencies for a fresh checkout
- `just dev-run`: backend + Vite frontend for local development
- `just dev-docker`: `docker compose up --build`
- `just run-docker`: packaged-app compose stack
- `just test`: full Python test suite plus frontend unit tests
- `just deliverable`: Dockerized wheel + sdist packaging workflow
- `just install`: install the packaged wheel into `~/.spark/venv` and initialize the stable runtime
- `just install-systemd`: Linux-only install flow that registers the packaged app as a `systemd --user` service

## Testing

Backend suite:

```bash
uv run pytest -q
```

Frontend unit tests:

```bash
npm --prefix frontend run test:unit
```

## Packaging

From a source checkout, `just dev-run` remains the canonical development path.
For distributable artifacts, use the Dockerized deliverable workflow:

```bash
just deliverable
```

`just deliverable` builds `Dockerfile.wheel`, which copies the source tree into the builder image and runs `uv run python scripts/build_deliverable.py` from that internal `/src` tree. The host only provides Docker build context and a mounted `dist/` directory for final artifact output. The builder compiles `frontend/dist`, stages the bundled UI into a temporary packaging tree, builds the wheel and sdist with standard setuptools, verifies the wheel contents, and copies the artifacts into `dist/`. The resulting `dist/` directory receives exactly one `spark-*.whl` wheel and one `spark-*.tar.gz` source distribution.

`Dockerfile.wheel` is only the deliverable builder environment; the root `Dockerfile` and `compose.package.yaml` remain the packaged application runtime surfaces.

Install the resulting wheel:

```bash
pip install dist/*.whl
```

On Linux, start the installed package as a background user service with:

```bash
spark-server service install
```

## Notes

- Flow files are stored as canonical DOT and validated before save.
- Spark flow self-description lives in DOT via `spark.title` and `spark.description`, while workspace launch policy is stored separately in `~/.spark/config/flow-catalog.toml`.
- Inside the assistant runtime, the Spark agent control surface uses bare `spark` commands such as `spark flow list`, `spark flow describe --flow <name>`, `spark flow get --flow <name>`, `spark flow validate --file <path> --text`, `spark convo run-request ...`, and `spark run launch ...`. In a human source-checkout shell, use `uv run spark ...` instead.
- The editor supports both structured editing and raw DOT editing, including semantic-equivalence safety checks during handoff.
- The Runs view is intended for historical inspection, diagnostics, artifact browsing, and replaying execution context.
- Packaged example flows live in [src/spark/flows/examples/simple-linear.dot](src/spark/flows/examples/simple-linear.dot), [src/spark/flows/examples/implement-review-loop.dot](src/spark/flows/examples/implement-review-loop.dot), [src/spark/flows/examples/human-review-loop.dot](src/spark/flows/examples/human-review-loop.dot), [src/spark/flows/examples/parallel-review.dot](src/spark/flows/examples/parallel-review.dot), [src/spark/flows/examples/supervision/implementation-worker.dot](src/spark/flows/examples/supervision/implementation-worker.dot), and [src/spark/flows/examples/supervision/supervised-implementation.dot](src/spark/flows/examples/supervision/supervised-implementation.dot).
- Packaged workflows live in [src/spark/flows/software-development/implement-change-request.dot](src/spark/flows/software-development/implement-change-request.dot), [src/spark/flows/software-development/spec-implementation/implement-spec.dot](src/spark/flows/software-development/spec-implementation/implement-spec.dot), and [src/spark/flows/software-development/spec-implementation/implement-milestone.dot](src/spark/flows/software-development/spec-implementation/implement-milestone.dot).
- Repo-only advanced/test fixtures live under [tests/fixtures/flows/](tests/fixtures/flows).

## Project Status

Active development.
