# Spark

Spark is a Cargo-built workspace workbench for AI-assisted software delivery. It ships the public native command names `spark` and `spark-server`, serves a React UI, and runs DOT-authored workflows through Rust crates.

## Install From Source

Build and install the native commands from a checkout:

```bash
cargo install --path crates/spark-cli --bin spark
cargo install --path crates/spark-server --bin spark-server
```

Initialize Spark and start the server:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

On Linux, manage the background user service with:

```bash
spark-server service install
spark-server service status
spark-server service remove
```

By default, runtime data lives under `~/.spark`. Set `SPARK_HOME` to use a different runtime tree.

## Development

Prepare the frontend toolchain and Cargo dependencies:

```bash
just setup
```

Run the backend and frontend together from a source checkout:

```bash
just dev-run
```

The backend uses `SPARK_HOME=~/.spark-dev` by default and serves on `127.0.0.1:8010`. The Vite frontend proxies to that backend unless `VITE_BACKEND_URL` is set.

Build the frontend and release binaries:

```bash
just deliverable
```

## Validation

The default repository gate is:

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features
npm --prefix frontend run test:unit
npm --prefix frontend run build
```

`just test` runs the full cutover validation gate: Rust formatting, Cargo tests, frontend unit tests, and the frontend production build.

## Runtime Assets

Runtime package data is Rust-owned:

- Packaged flows: [crates/spark-assets/assets/flows](crates/spark-assets/assets/flows)
- Packaged guides: [crates/spark-assets/assets/guides](crates/spark-assets/assets/guides)
- Model catalog: [crates/spark-assets/assets/unified_llm/data/models.json](crates/spark-assets/assets/unified_llm/data/models.json)
- Shared Rust test fixtures: [crates/test-fixtures](crates/test-fixtures)
- Frontend source: [frontend](frontend)

The asset crate embeds authored flows, guides, the model catalog, icons, and the built frontend. No Python package tree, wheel, sdist, or virtual environment is part of the supported runtime or install path.

## Requirements

- Rust stable toolchain with Cargo
- Node.js 20+ and npm
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows
- `just` for the documented convenience commands

## Provider Configuration

Provider keys for service and foreground runs should live outside project repositories in `$SPARK_HOME/config/provider.env`:

```bash
mkdir -p ~/.spark/config
chmod 700 ~/.spark/config
$EDITOR ~/.spark/config/provider.env
chmod 600 ~/.spark/config/provider.env
```

Supported provider variables include `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, `OPENROUTER_TITLE`, `LITELLM_BASE_URL`, `LITELLM_API_KEY`, `OPENAI_COMPATIBLE_BASE_URL`, and `OPENAI_COMPATIBLE_API_KEY`.
