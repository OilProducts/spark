# Spark

Spark is a workspace workbench for AI-assisted software delivery.

It packages:

- a FastAPI backend for running Attractor workflows
- a bundled web UI for flow authoring, execution, and run inspection
- bundled `.dot` flows and packaged authoring/operations guides
- CLIs for launching the server and interacting with workspace conversations

## Install

```bash
pip install spark
```

## Quick Start

Initialize a Spark home and seed the bundled flows:

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
- `spark-server init`: initialize runtime directories and seed bundled flows
- `spark`: workspace conversation, run-launch, flow, and trigger commands

## Requirements

- Python 3.10+
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows

## Package Contents

The supported install artifacts are the wheel and sdist.
Both include:

- bundled UI assets under `spark/ui_dist`
- packaged flows under `spark/flows`, including examples under `spark/flows/examples`
- packaged guidance docs under `spark/guides`, including `dot-authoring.md` and `spark-operations.md`

## Development

The source repository includes the React frontend, tests, specs, and local development tooling. For local development, use the repository README instead of this package README.
