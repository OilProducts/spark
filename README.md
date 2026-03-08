# Spark Spawn

Spark Spawn is a DOT-driven workflow runner for multi-stage AI pipelines, with a FastAPI backend and React UI for authoring, execution, and run inspection.

## Description

Spark Spawn lets you define workflows as directed graphs (DOT), run them with built-in handlers (codergen, tool, conditional, fan-in, parallel, human gate), and inspect status, events, checkpoints, context, and artifacts from the web UI.

## Features

- DOT-first workflow model with parser, formatter, and validator
- Live run monitoring with SSE and run history
- Human-in-the-loop question/answer flow over HTTP
- Artifact browsing, including Graphviz pipeline render output
- Wheel distribution with `sparkspawn` CLI entrypoint

## Installation

Build and install from source:

```bash
npm --prefix frontend run build
./scripts/sync_ui_dist.sh
uv build
pip install dist/sparkspawn-0.1.0-py3-none-any.whl
```

Runtime prerequisites:

- `codex` CLI on `PATH` (for codergen backends)
- Graphviz `dot` on `PATH` (for graph SVG artifacts)

## Usage

Start the server:

```bash
sparkspawn serve --host 127.0.0.1 --port 8000
```

For local full-stack development without Docker:

```bash
just run
```

This starts the backend on `127.0.0.1:8000` and the Vite frontend dev server with API proxying enabled.
The backend runs with auto-reload enabled in this mode, so Python/API changes are picked up without restarting `just run`.
If the backend cannot bind to `127.0.0.1:8000`, `sparkspawn serve` reports that failure directly and `just run` tears down the frontend process.

Optional runtime path overrides (CLI args or env vars):

- `SPARKSPAWN_HOME`
- `SPARKSPAWN_RUNS_DIR`
- `SPARKSPAWN_FLOWS_DIR`
- `SPARKSPAWN_UI_DIR`

Open: [http://127.0.0.1:8000](http://127.0.0.1:8000)

## API

Key endpoints:

- `POST /pipelines`
- `GET /pipelines/{id}`
- `GET /pipelines/{id}/events`
- `GET /pipelines/{id}/graph`
- `GET /pipelines/{id}/checkpoint`
- `GET /pipelines/{id}/context`
- `GET /runs`
- `GET /api/flows`, `POST /api/flows`, `GET /api/flows/{name}`, `DELETE /api/flows/{name}`

## Testing

```bash
uv run pytest -q
npm --prefix frontend run test:unit
```

## Roadmap

- Improve packaging ergonomics (release workflow, docs polish)
- Add runtime `doctor` checks for external binary dependencies
- Continue UI parity and contract hardening

## Contributing

- Fork and create a feature branch.
- Keep changes scoped and test-backed.
- Run `uv run pytest -q` before opening a PR.

## Support

Open an issue in this repository with reproduction steps, expected behavior, and logs/screenshots where relevant.

## Acknowledgments

- [FastAPI](https://fastapi.tiangolo.com/)
- [React Flow](https://reactflow.dev/)
- [Graphviz](https://graphviz.org/)

## Project Status

Active development.
