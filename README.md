# Spark

Spark is a workspace workbench for AI-assisted software delivery. It now uses Rust-backed `spark` and `spark-server` binaries with a React UI for registering local projects, authoring shared DOT workflows, running them against a selected project context, and coordinating project conversations, flow requests, and run launches.

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

When working from a source checkout, validate direct flow edits locally with `uv run spark flow validate --file /path/to/flow.dot --text`. Server-backed `spark` commands from the checkout should set `SPARK_API_BASE_URL=http://127.0.0.1:8010` when they target the development backend.

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

Spark is distributed through the public command names `spark` and `spark-server`. Packaged installs execute native Rust binaries shipped with the wheel.

In source checkouts, `uv run spark` and `uv run spark-server` are development entry points for the same command surface: they dispatch to packaged Rust payloads when present, otherwise to built workspace binaries under `target/debug` or `target/release`. If those binaries are unavailable, the launchers fail with `cargo build` instructions instead of entering Python Spark CLI, server, or provider fallback paths. Source-checkout runtime state stays separated from a stable install with `SPARK_HOME=~/.spark-dev` and `SPARK_API_BASE_URL=http://127.0.0.1:8010`. Normal Spark server and CLI unified LLM provider execution remains Rust-owned through the Rust crates and adapters; Python `unified_llm` provider clients are retained for compatibility/oracle/package-data support, not as the normal provider runtime.

Normal Spark chat, `agent-turn`, and codergen-adjacent agent execution are Rust-owned through [crates/spark-agent-adapter/](crates/spark-agent-adapter). Workspace conversation APIs and live delivery keep the public HTTP and SSE shapes, while the Python facade in [src/spark/chat/session.py](src/spark/chat/session.py) serializes requests to the `spark-agent-boundary` Rust binary and maps the returned payloads back to persisted conversation records. The retained [src/agent/](src/agent) package is compatibility, oracle, fallback, and historical implementation support only; it is not the normal runtime implementation for Spark chat or agent execution.

The migration source of truth is [docs/rust-rewrite-migration.md](docs/rust-rewrite-migration.md). It records the distribution-readiness boundary, validation evidence, retained Python compatibility status, and explicitly out-of-scope extensions such as MCP, skills, new sandbox policy systems, automatic compaction, new approval systems, and read-before-write guardrails unless those features have their own implemented evidence.

- [Cargo.toml](Cargo.toml): Rust workspace manifest for the Spark rewrite
- [crates/spark-cli/](crates/spark-cli): Rust `spark` command surface for conversations, runs, flows, and triggers
- [crates/spark-server/](crates/spark-server): Rust `spark-server` command surface for serving, init, service management, and hidden worker execution
- [crates/spark-http/](crates/spark-http), [crates/spark-workspace/](crates/spark-workspace), and [crates/attractor-api/](crates/attractor-api): composed HTTP app, Workspace API, and Attractor API ownership
- [crates/attractor-dsl/](crates/attractor-dsl), [crates/attractor-core/](crates/attractor-core), and [crates/attractor-runtime/](crates/attractor-runtime): DOT parsing, typed graph/runtime contracts, and execution state machine
- [crates/spark-assets/](crates/spark-assets) and [crates/spark-storage/](crates/spark-storage): packaged resource discovery and compatible filesystem storage
- [crates/spark-agent-adapter/](crates/spark-agent-adapter) and [crates/unified-llm-adapter/](crates/unified-llm-adapter): Rust-owned adapter boundaries for Codex/agent and provider/model behavior during migration
- [frontend/](frontend): React 19 + Vite UI served by the Rust-backed server
- [src/spark/flows/](src/spark/flows): packaged `.dot` flows shipped with Spark; examples live under [src/spark/flows/examples/](src/spark/flows/examples)
- [tests/fixtures/flows/](tests/fixtures/flows): repo-only `.dot` fixtures used by tests and local development
- [tests/](tests): Rust parity fixtures, Python guardrails, UI contracts, and acceptance assets
- [specs/](specs): Attractor, workspace, frontend, and storage specifications

### Agent Host Controls

The Rust agent adapter boundary preserves Spark adapter payloads, event streams, token usage, raw logs, request-user-input records, and failure payloads across the Workspace, HTTP, Python facade, and Attractor codergen integration boundaries:

- Tool results sent back to the model are truncated by per-tool character limits before line limits. The visible truncation marker states what was omitted and that full output is available through the event stream. `TOOL_CALL_END` keeps the full untruncated output or error data and also exposes the model-visible `model_content`.
- System prompts are assembled from provider base instructions, environment context, active tool descriptions, project instructions, and user overrides. The environment snapshot is captured at session start and includes working directory, git state, branch, platform, OS version, date, model display name, knowledge cutoff, modified/untracked counts, and recent commits.
- Project instruction discovery walks from git root or working directory to the active working directory. `AGENTS.md` is always considered; only the active provider file is included among `.codex/instructions.md`, `CLAUDE.md`, and `GEMINI.md`; project instructions are capped at 32 KB with a truncation marker.
- Provider configuration and selectors are normalized before crossing the Rust boundary. Supported selectors are `codex`, `openai`, `anthropic`, `gemini`, `openrouter`, `litellm`, and `openai_compatible`; `compatible` is normalized to `openai_compatible`, profile IDs preserve request/session identity, and profile-backed calls dispatch through the configured provider. `reasoning_effort` values such as `low`, `medium`, `high`, and null are normalized and reflected in the next unified LLM `Request` and provider-specific options.
- Native OpenAI, Anthropic, and Gemini coding profiles use provider-specific editing surfaces: OpenAI uses the Codex-style `apply_patch` path for targeted edits, Anthropic uses `edit_file` old/new-string replacements for existing files, and Gemini uses its `edit_file` search-and-replace shape. OpenAI-compatible, OpenRouter, and LiteLLM use the OpenAI-compatible tool-calling path rather than claiming native Anthropic or Gemini behavior.
- Context warnings use the approximate one-token-per-four-characters heuristic and emit `WARNING` events above 80 percent of the provider context window. They do not automatically compact or summarize history.
- `session.steer` queues a `SteeringTurn` for the next model call, including after the current tool round. Steering is emitted as `STEERING_INJECTED` and is converted to a user-role request message without restarting history.
- `session.follow_up` queues another user input after natural completion on the same history and event stream. It is not processed after abort or unrecoverable close.
- Attractor codergen agent mode uses the same Rust session boundary for tool-enabled turns. Child intervention requests are delivered as steering to the active Rust codergen session when the child run, root run, and target node match; stale, inactive, or mismatched interventions are rejected as unsupported steering instead of mutating a different session.
- Execution environments are typed and swappable at the adapter boundary. Local sessions and child sessions share the selected backend, scoped child working directories reject path escape before registration, and future environment backends must preserve the same observable file, command, tool, and event contracts.
- Recoverable tool errors remain model-visible tool results and stream events. Unrecoverable session failures emit `ERROR`, close or abort the session, and prevent queued follow-up processing; thread resume failures are preserved in the serialized agent-turn output.
- Turn and tool-round limits emit `TURN_LIMIT`; repeated tool-call signatures of pattern length 1, 2, or 3 emit `LOOP_DETECTION` and add recovery steering without silently changing assistant output.

The source-checkout validation gate remains:

```bash
uv run pytest -q
```

Focused retained-Python compatibility checks live in `tests/agent/test_truncation.py`, `tests/agent/test_prompts.py`, `tests/agent/test_project_docs.py`, `tests/agent/test_context_usage.py`, `tests/agent/test_steering.py`, `tests/agent/test_reasoning_effort.py`, `tests/agent/test_loop_detection.py`, and `tests/compat/agent`. They do not make `src/agent` the normal runtime owner. Rust adapter behavior is validated by contract tests under `crates/spark-agent-adapter/tests`, including runtime, event, provider/profile, prompt context, tool dispatch, local environment, and codergen coverage. Pytest compatibility checks also expose focused Rust Workspace, HTTP, and SSE contract tests for the active adapter boundary, while direct Rust validation remains available with the crate-specific `cargo test` commands documented in the migration guide.

## Requirements

- Rust stable toolchain with Cargo
- Python 3.11+
- [`uv`](https://docs.astral.sh/uv/)
- Node.js 20+ and npm
- Graphviz `dot` on `PATH` for graph artifacts
- `codex` CLI on `PATH` with working auth for Codex-backed handlers and project chat flows
- `just` is optional, but the repo commands assume it when available

Python and `uv` remain part of the source-checkout workflow for development command wrappers, compatibility guardrails, tests, packaging, oracle fixture generation, and package data. They do not change the supported user commands or make Python `unified_llm` provider clients the normal Spark server or CLI provider runtime.

## Local Development

Prepare a fresh checkout:

```bash
just setup
```

This installs the Python development environment with `uv sync --dev` and the frontend toolchain with `npm --prefix frontend ci`. Build the Rust workspace directly when you need fresh local binaries:

```bash
cargo build --workspace
```

`uv run spark --help` and `uv run spark-server --help` dispatch to the built workspace binaries from a source checkout. If the binaries are absent, build the specific command first:

```bash
cargo build -p spark-cli --bin spark
cargo build -p spark-server --bin spark-server
```

Initialize the runtime tree and seed packaged flows:

```bash
SPARK_HOME=~/.spark-dev uv run spark-server init
```

Source-checkout commands should use a separate development runtime so they do not mutate a stable install under `~/.spark`.

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

### LLM Profiles

Custom OpenAI-compatible model endpoints are configured in `$SPARK_HOME/config/llm-profiles.toml`. Public profile metadata exposes profile id, label, provider, models, default model, and configured status; endpoint URLs, API key environment names, and secret values stay redacted.

Example:

```toml
[profiles.local-qwen]
label = "Local Qwen"
provider = "openai_compatible"
base_url = "http://127.0.0.1:11434/v1"
models = ["qwen2.5-coder"]
default_model = "qwen2.5-coder"

[profiles.team-router]
label = "Team Router"
provider = "openai_compatible"
base_url = "https://llm-gateway.example.com/v1"
api_key_env = "TEAM_ROUTER_API_KEY"
models = ["team-coder"]
default_model = "team-coder"
```

Profile selection is preserved as request and session identity while low-level calls dispatch through the configured provider. Omitting `api_key_env` creates a local no-key profile; profiles with `api_key_env` require that process environment variable to be non-empty. Chat/session payloads, flow defaults such as `ui_default_llm_profile`, launch context keys, and supported recovery CLI/API commands can carry profile, provider, model, and `reasoning_effort` controls; ordinary provider secrets still come from the Spark server process environment.

Run the full stack locally:

```bash
just dev-run
```

This starts:

- the backend on `127.0.0.1:8010`
- the Vite frontend on `127.0.0.1:5173`

Open [http://127.0.0.1:5173](http://127.0.0.1:5173) for live frontend development.

The source-checkout development entry points intentionally use a separate runtime home and port so they do not stomp on a stable installed Spark instance:

- `SPARK_HOME` defaults to `~/.spark-dev`
- backend port defaults to `8010`
- frontend port defaults to `5173`
- server-backed checkout CLI calls should use `SPARK_API_BASE_URL=http://127.0.0.1:8010`
- explicit development data, flow, and UI directories can be supplied with `--data-dir`, `--flows-dir`, `--ui-dir`, `SPARK_FLOWS_DIR`, and `SPARK_UI_DIR`
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

For the packaged Docker runtime:

```bash
just run-docker
```

That stack keeps the container's internal Spark home at `/spark`, but bind-mounts it from `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}` on the host.
Packaged Docker state therefore lives under `~/.spark-docker` by default, seeded packaged flows appear at `~/.spark-docker/flows`, and provider secrets for that runtime belong in `~/.spark-docker/config/provider.env`.
The packaged container runs as the launching user's UID/GID so files written under the host-mounted Spark home remain editable by that user.
On first launch through `just run-docker`, Codex auth and config are seeded from `${CODEX_HOME:-$HOME/.codex}` into `~/.spark-docker/runtime/codex/.codex/` when `auth.json` or `config.toml` exist on the host and are not already present in the packaged Docker home.
Existing packaged Docker Codex auth and config files are preserved, so that runtime can keep separate credentials after initialization.
Use a different host location with:

```bash
SPARK_DOCKER_HOME=/some/path just run-docker
```

Raw `docker compose -f compose.package.yaml up --build` does not perform this host-side Codex seeding; `just run-docker` is the supported packaged Docker launcher.
The native packaged install continues to use `~/.spark`, and source-checkout development continues to use `~/.spark-dev` by default.
The packaged Docker stack also keeps project work mounted from `${SPARK_PROJECTS_HOST_DIR:-$HOME/projects}` into `/projects`.

## Backend-Only Usage

Start the server directly:

```bash
SPARK_HOME=~/.spark-dev uv run spark-server serve --host 127.0.0.1 --port 8010
```

Useful development flags:

```bash
SPARK_HOME=~/.spark-dev \
uv run spark-server serve \
  --host 127.0.0.1 \
  --port 8010 \
  --reload \
  --data-dir ~/.spark-dev \
  --flows-dir ~/.spark-dev/flows \
  --ui-dir ./frontend/dist
```

When a built UI is available, the development backend serves it at [http://127.0.0.1:8010](http://127.0.0.1:8010). The stable packaged runtime continues to default to [http://127.0.0.1:8000](http://127.0.0.1:8000).

## Runtime Data and Configuration

By default, packaged Spark stores runtime data under `~/.spark`; source-checkout development should use `SPARK_HOME=~/.spark-dev`:

- `config/`
- `runtime/`
- `logs/`
- `workspace/projects/`
- `attractor/runs/`
- `flows/`

Important path overrides:

- `SPARK_HOME`
- `SPARK_API_BASE_URL`
- `SPARK_FLOWS_DIR`
- `SPARK_UI_DIR`

Spark-managed Codex runtime state defaults to `~/.spark/runtime/codex`, or `<SPARK_HOME>/runtime/codex` when `SPARK_HOME` is set. Set `ATTRACTOR_CODEX_RUNTIME_ROOT` only when you need to force a different runtime root.

`~/.spark/config/prompts.toml` stores user-configurable prompt templates and is created on first startup.

### Execution Profiles

Spark selects where runs execute through profiles in `~/.spark/config/execution-profiles.toml`. Supported profile modes are:

- `native`: run directly in the Spark server environment.
- `local_container`: run each node through the local container runner. These profiles must set `image`.

When `execution-profiles.toml` is absent and no profile is explicitly selected, Spark synthesizes a native profile named `native`. If a project, workspace default, chat approval, CLI call, or launch API selects a profile, that profile must exist in the config file and be enabled.

Example:

```toml
[defaults]
execution_profile_id = "local-dev"

[profiles.native-dev]
mode = "native"
label = "Native Dev"

[profiles.local-dev]
mode = "local_container"
label = "Local Container Dev"
image = "spark-exec:latest"
enabled = true
capabilities = ["network"]
metadata = { owner = "platform" }
```

Profile fields:

- `mode` and `label` are required for every configured profile.
- `image` is required for `local_container` profiles and ignored for `native` profiles.
- `enabled` is optional and defaults to `true`; disabled profiles remain visible but cannot be selected.
- `capabilities` is an optional array of non-empty strings.
- `metadata` is an optional table passed through with the profile.

The config shape is `[defaults]` plus `[profiles.<profile-id>]` tables. Worker tables such as `[workers.*]` are not part of the supported execution profile contract.

## API Overview

For a task-oriented packaged reference that pairs Spark CLI commands with the matching HTTP routes, use [src/spark/guides/spark-operations.md](src/spark/guides/spark-operations.md).

The canonical route inventory lives in [app.py](src/spark/app.py), [server.py](src/attractor/api/server.py), and [api.py](src/spark/workspace/api.py).

The root app is a mount host only. Canonical public API surfaces are:
- Attractor docs/OpenAPI under `/attractor/docs` and `/attractor/openapi.json`
- Workspace docs/OpenAPI under `/workspace/docs` and `/workspace/openapi.json`

Current API groups include:

- Attractor runtime and runs: `GET /attractor/status`, `GET /attractor/runs`
- Attractor pipeline execution: `POST /attractor/pipelines`, `GET /attractor/pipelines/{id}`, `POST /attractor/pipelines/{id}/cancel`
- Attractor pipeline inspection: `GET /attractor/pipelines/{id}/journal`, `GET /attractor/pipelines/{id}/checkpoint`, `GET /attractor/pipelines/{id}/context`, `GET /attractor/pipelines/{id}/graph`, `GET /attractor/pipelines/{id}/artifacts`
- Attractor human-gate actions: `GET /attractor/pipelines/{id}/questions`, `POST /attractor/pipelines/{id}/questions/{question_id}/answer`
- Attractor flow management: `GET /attractor/api/flows`, `POST /attractor/api/flows`, `GET /attractor/api/flows/{name}`, `DELETE /attractor/api/flows/{name}`
- Workspace project management: `GET /workspace/api/projects`, `POST /workspace/api/projects/register`, `PATCH /workspace/api/projects/state`, `DELETE /workspace/api/projects`
- Workspace triggers and project metadata: `GET /workspace/api/triggers`, `POST /workspace/api/triggers`, `GET /workspace/api/triggers/{trigger_id}`, `PATCH /workspace/api/triggers/{trigger_id}`, `DELETE /workspace/api/triggers/{trigger_id}`, `POST /workspace/api/webhooks`, `GET /workspace/api/projects/metadata`, `GET /workspace/api/projects/browse`
- Workspace live delivery: `GET /workspace/api/live/events` with resource filters such as `conversation_id` plus required `conversation_project_path`, selected-run `run_id`, `include_runs_overview` plus optional active-project `runs_project_path`, `include_triggers` plus `triggers_project_path`, and reconnect/catch-up cursors such as `conversation_revision` and `run_sequence`
- Workspace conversations: `GET /workspace/api/projects/conversations`, `GET /workspace/api/conversations/{conversation_id}`, `GET /workspace/api/conversations/{conversation_id}/events`, `POST /workspace/api/conversations/{conversation_id}/turns`, `DELETE /workspace/api/conversations/{conversation_id}`
- Workspace run-launch workflows: `POST /workspace/api/conversations/by-handle/{conversation_handle}/flow-run-requests`, `POST /workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review`, `POST /workspace/api/runs/launch`, `POST /workspace/api/runs/{run_id}/retry`, `POST /workspace/api/runs/{run_id}/continue`

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

Rust workspace checks:

```bash
cargo fmt --check
cargo test --workspace --all-features
```

Backend suite:

```bash
uv run pytest -q
```

Optional live smoke tests are not part of ordinary validation. The Rust provider smoke harness in `crates/unified-llm-adapter/tests/live_smoke_contracts.rs` runs only when `UNIFIED_LLM_ADAPTER_RUN_LIVE=1` is set and provider credentials are present in the process environment. Retained Python adapter smoke tests require explicit pytest live selection such as `uv run pytest -q --run-live tests/adapters/test_cross_provider_parity.py`.

Frontend checks:

```bash
npm --prefix frontend run test:unit
npm --prefix frontend run build
npm --prefix frontend run ui:smoke
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
- Inside the assistant runtime, the Spark agent control surface uses bare `spark` commands such as `spark flow list`, `spark flow describe --flow <name>`, `spark flow get --flow <name>`, `spark flow validate --file <path> --text`, `spark convo run-request ...`, `spark run launch ...`, `spark run retry ...`, and `spark run continue ...`. In a human source-checkout shell, use `uv run spark ...` instead.
- The editor supports both structured editing and raw DOT editing, including semantic-equivalence safety checks during handoff.
- The Runs view is intended for historical inspection, diagnostics, artifact browsing, and replaying execution context.
- Packaged example flows live in [src/spark/flows/examples/simple-linear.dot](src/spark/flows/examples/simple-linear.dot), [src/spark/flows/examples/implement-review-loop.dot](src/spark/flows/examples/implement-review-loop.dot), [src/spark/flows/examples/human-review-loop.dot](src/spark/flows/examples/human-review-loop.dot), [src/spark/flows/examples/parallel-review.dot](src/spark/flows/examples/parallel-review.dot), [src/spark/flows/examples/supervision/implementation-worker.dot](src/spark/flows/examples/supervision/implementation-worker.dot), and [src/spark/flows/examples/supervision/supervised-implementation.dot](src/spark/flows/examples/supervision/supervised-implementation.dot).
- Packaged workflows live in [src/spark/flows/software-development/implement-change-request.dot](src/spark/flows/software-development/implement-change-request.dot), [src/spark/flows/software-development/spec-implementation/implement-spec.dot](src/spark/flows/software-development/spec-implementation/implement-spec.dot), and [src/spark/flows/software-development/spec-implementation/implement-milestone.dot](src/spark/flows/software-development/spec-implementation/implement-milestone.dot).
- Repo-only advanced/test fixtures live under [tests/fixtures/flows/](tests/fixtures/flows).

## Project Status

Active development.
