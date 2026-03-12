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

- [attractor/](/Users/chris/tinker/sparkspawn/attractor): FastAPI server, CLI, DOT parser/validator, execution engine, handlers, runtime storage
- [frontend/](/Users/chris/tinker/sparkspawn/frontend): React 19 + Vite UI
- [flows/](/Users/chris/tinker/sparkspawn/flows): sample and reference `.dot` flows, including planning flows
- [tests/](/Users/chris/tinker/sparkspawn/tests): backend tests, UI contracts, and acceptance assets
- [specs/](/Users/chris/tinker/sparkspawn/specs): product notes and workflow references

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
  --flows-dir ./flows \
  --ui-dir ./frontend/dist
```

When a built UI is available, the backend serves it at [http://127.0.0.1:8000](http://127.0.0.1:8000).

## Runtime Data and Configuration

By default, Spark Spawn stores runtime data under `~/.sparkspawn`:

- `config/`
- `runtime/`
- `logs/`
- `projects/`
- `flows/`

Important path overrides:

- `SPARKSPAWN_HOME`
- `SPARKSPAWN_FLOWS_DIR`
- `SPARKSPAWN_UI_DIR`

`~/.sparkspawn/config/prompts.toml` stores user-configurable prompt templates and is created on first startup.

## API Overview

The canonical route inventory lives in [attractor/api/server.py](/Users/chris/tinker/sparkspawn/attractor/api/server.py). Current API groups include:

- Runtime and runs: `GET /status`, `GET /runs`
- Pipeline execution: `POST /pipelines`, `POST /run`, `GET /pipelines/{id}`, `POST /pipelines/{id}/cancel`
- Pipeline inspection: `GET /pipelines/{id}/events`, `GET /pipelines/{id}/checkpoint`, `GET /pipelines/{id}/context`, `GET /pipelines/{id}/graph`, `GET /pipelines/{id}/artifacts`
- Human-gate actions: `GET /pipelines/{id}/questions`, `POST /pipelines/{id}/questions/{question_id}/answer`
- Flow management: `GET /api/flows`, `POST /api/flows`, `GET /api/flows/{name}`, `DELETE /api/flows/{name}`
- Project management: `GET /api/projects`, `POST /api/projects/register`, `PATCH /api/projects/state`, `DELETE /api/projects`
- Project metadata and directory selection: `GET /api/projects/metadata`, `POST /api/projects/pick-directory`
- Project conversations: `GET /api/projects/conversations`, `GET /api/conversations/{conversation_id}`, `GET /api/conversations/{conversation_id}/events`, `POST /api/conversations/{conversation_id}/turns`, `DELETE /api/conversations/{conversation_id}`
- Review workflows: `POST /api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/approve`, `POST /api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/reject`, `POST /api/conversations/{conversation_id}/execution-cards/{execution_card_id}/review`

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
- The editor supports both structured editing and raw DOT editing, including semantic-equivalence safety checks during handoff.
- The Runs view is intended for historical inspection, diagnostics, artifact browsing, and replaying execution context.
- Example planning flows live in [flows/plan-generation.dot](/Users/chris/tinker/sparkspawn/flows/plan-generation.dot) and [flows/implement-spec.dot](/Users/chris/tinker/sparkspawn/flows/implement-spec.dot).

## Project Status

Active development.
