# Spark

Spark is a workspace workbench for AI-assisted software delivery.

It packages Rust-backed `spark` and `spark-server` commands with:

- a bundled server for running Spark Workspace and Attractor workflows
- a bundled web UI for flow authoring, execution, and run inspection
- bundled `.dot` flows and packaged authoring/operations guides
- CLIs for launching the server and interacting with workspace conversations

## Install

```bash
pip install spark
```

## Quick Start

On Linux, initialize a Spark home, seed the bundled flows, install a `systemd --user` unit, and start Spark in the background:

```bash
spark-server service install
```

This serves the bundled UI at `http://127.0.0.1:8000`.

To listen on every interface instead, install the service with an explicit bind host:

```bash
spark-server service install --host 0.0.0.0 --port 8000
```

Inspect or remove the service with:

```bash
spark-server service status
spark-server service remove
```

By default, Spark stores runtime data under `~/.spark` and serves the bundled UI when no external UI directory is configured.

If you prefer a foreground process instead of a user service:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

## Included Commands

- `spark-server serve`: start the Spark API server
- `spark-server init`: initialize runtime directories and seed bundled flows
- `spark-server service install|remove|status`: manage the Linux user service
- `spark`: workspace conversation, run-launch, flow, and trigger commands

The command names are the compatibility contract. Package entry points dispatch to the native Rust binaries included with the wheel while preserving the documented `spark` and `spark-server` user surface.

## Requirements

- Python 3.11+
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows

Rust and Node.js are build-time requirements for maintainers working from the source repository, not runtime requirements for normal wheel users.

## Runtime Configuration

The packaged runtime uses `~/.spark` unless `SPARK_HOME` is set. Provider keys for service and foreground runs should live outside project repositories in `$SPARK_HOME/config/provider.env`:

```bash
mkdir -p ~/.spark/config
chmod 700 ~/.spark/config
$EDITOR ~/.spark/config/provider.env
chmod 600 ~/.spark/config/provider.env
```

Supported provider variables include `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, `OPENROUTER_TITLE`, `LITELLM_BASE_URL`, and `LITELLM_API_KEY`.

The Linux user service loads that file through `EnvironmentFile=-$SPARK_HOME/config/provider.env`. Keep API keys out of flow DOT, launch context, project files, and checked-in configuration.

## Package Contents

The supported install artifacts are the wheel and sdist.
Both include:

- native command payloads under `spark/bin`
- bundled UI assets under `spark/ui_dist`
- packaged flows under `spark/flows`, including examples under `spark/flows/examples`
- packaged guidance docs under `spark/guides`, including `dot-authoring.md` and `spark-operations.md`

## Development

The source repository includes the Rust workspace, React frontend, Python compatibility tests, specs, and local development tooling. For local development, use the repository README instead of this package README. Maintainer validation includes `cargo fmt --check`, `cargo test --workspace --all-features`, `uv run pytest -q`, `npm --prefix frontend run test:unit`, `npm --prefix frontend run build`, and `npm --prefix frontend run ui:smoke`.
