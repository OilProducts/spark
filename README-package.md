# Spark

Spark is a workspace workbench for AI-assisted software delivery.

It packages:

- a FastAPI backend for running Attractor workflows
- a bundled web UI for flow authoring, execution, and run inspection
- curated starter `.dot` flows and authoring guides
- CLIs for launching the server and interacting with workspace conversations

## Install

```bash
pip install spark
```

## Quick Start

Initialize a Spark home and seed the starter flows:

```bash
spark-server init
```

Start the server:

```bash
spark-server serve --host 127.0.0.1 --port 8000
```

By default, Spark stores runtime data under `~/.spark` and serves the bundled UI when no external UI directory is configured.

## Included Commands

- `spark-server serve`: start the Spark API server
- `spark-server init`: initialize runtime directories and seed starter flows
- `spark-server release-gate`: validate the required UI checklist
- `spark`: workspace conversation, run-launch, flow, and trigger commands

## Requirements

- Python 3.10+
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows

## Package Contents

The wheel includes:

- bundled UI assets under `spark_app/ui_dist`
- starter flows under `spark/starter_flows`
- packaged authoring guides under `spark/guides`

## Development

The source repository includes the React frontend, tests, specs, and local development tooling. For local development, use the repository README instead of this package README.
