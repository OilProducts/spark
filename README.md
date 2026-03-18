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

The canonical route inventory lives in [server.py](/Users/chris/tinker/sparkspawn/src/attractor/api/server.py) and [api.py](/Users/chris/tinker/sparkspawn/src/workspace/api.py).

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
- Starter flow templates live in [starter-flows/plan-generation.dot](/Users/chris/tinker/sparkspawn/starter-flows/plan-generation.dot), [starter-flows/parallel-review.dot](/Users/chris/tinker/sparkspawn/starter-flows/parallel-review.dot), and [starter-flows/manager-human.dot](/Users/chris/tinker/sparkspawn/starter-flows/manager-human.dot).
- Repo-only advanced/test fixtures live under [tests/fixtures/flows/](/Users/chris/tinker/sparkspawn/tests/fixtures/flows).

## Project Status

Active development.
