# First Flow Tutorial

This tutorial is for a human user who wants to learn Spark flow authoring from the simplest case up through loops, launch inputs, and tool artifacts.

It is not the full reference. Use this document to get your first few flows working, then use the DOT reference in [src/spark/guides/dot-authoring.md](../src/spark/guides/dot-authoring.md) when you need the full attribute catalog.

## What You Will Build

You will go through four levels:

1. A simple linear flow.
2. A flow with structured launch inputs.
3. A flow with a review loop.
4. A flow with a tool node that preserves artifacts.

You can do every step in the visual editor, by editing the `.dot` file directly, or by mixing both approaches.

## Prerequisites

Install dependencies and seed the runtime tree with the source-checkout development wrapper:

```bash
just setup
SPARK_HOME=~/.spark-dev uv run spark-server init
```

The `uv run spark` and `uv run spark-server` commands are source-checkout development entry points for the packaged command surface. They keep the documented command syntax available while bootstrapping the checkout's Rust-owned server and CLI runtime path where the rewrite supports it. They do not make Python `unified_llm` provider clients the normal Spark server or CLI provider execution path.

If you need native Cargo-built binaries from the checkout, build the workspace and call the Cargo output paths directly:

```bash
cargo build --workspace
SPARK_HOME=~/.spark-dev target/debug/spark-server init
```

For the full editor and frontend during source development, start the app through the wrapper:

```bash
just dev-run
```

That gives you:

- the backend at `http://127.0.0.1:8010`
- the frontend at `http://127.0.0.1:5173`
- a local flow library at `~/.spark-dev/flows`

Use `~/.spark-dev` for source-checkout work so the tutorial does not mutate a stable packaged runtime under `~/.spark`. Server-backed checkout CLI commands should target the development backend with `SPARK_API_BASE_URL=http://127.0.0.1:8010`. Local file validation remains safe without a server target:

```bash
uv run spark flow validate --file ~/.spark-dev/flows/my-first-flow.dot --text
```

To exercise the Cargo-built Rust server binary directly, stop any server already using port `8010` and run the binary from `target/debug`:

```bash
SPARK_HOME=~/.spark-dev target/debug/spark-server serve \
  --host 127.0.0.1 \
  --port 8010 \
  --data-dir ~/.spark-dev \
  --flows-dir ~/.spark-dev/flows
```

## Part 1: Start From A Minimal Flow

Seeded packaged flows already live in `~/.spark-dev/flows`. Copy the smallest example so you can edit your own version:

```bash
cp ~/.spark-dev/flows/examples/simple-linear.dot ~/.spark-dev/flows/my-first-flow.dot
uv run spark flow validate --file ~/.spark-dev/flows/my-first-flow.dot --text
```

Open `my-first-flow.dot` in the Editor.

The copied starter is structurally equivalent to this complete flow:

```dot
digraph my_first_flow {
  graph [
    goal="Add a /healthz endpoint that returns {\"status\":\"ok\"}.",
    label="My First Flow",
    spark.title="My First Flow",
    spark.description="A minimal workflow for one small repository change."
  ];

  start [shape=Mdiamond, label="Start"];
  plan [shape=box, label="Plan", prompt="Inspect the repository and plan the work."];
  implement [shape=box, label="Implement", prompt="Make the requested change and keep it scoped."];
  summarize [shape=box, label="Summarize", prompt="Summarize the result and validation."];
  done [shape=Msquare, label="Done"];

  start -> plan -> implement -> summarize -> done;
}
```

That is the best first shape because each node has one job:

- `plan`: inspect and decide what to do
- `implement`: make the change
- `summarize`: explain what happened

### Set The Stated Goal

The flow-level `goal` is the run's stated goal. Start by making it concrete.

Example:

```dot
graph [
  goal="Add a /healthz endpoint that returns {\"status\":\"ok\"}.",
  label="My First Flow",
  spark.title="My First Flow",
  spark.description="A minimal workflow for one small repository change."
];
```

In the UI:

- `goal` is the stated goal for the run
- `spark.title` and `spark.description` are the flow's catalog metadata

Validate after each meaningful edit:

```bash
uv run spark flow validate --file ~/.spark-dev/flows/my-first-flow.dot --text
```

## Part 2: Add Launch Inputs

One goal string is often not enough. Use launch inputs when you need structured information such as target files or acceptance criteria.

You can author them in Graph Settings -> Launch Inputs, or directly in DOT with `spark.launch_inputs`.

Example:

```dot
graph [
  goal="Implement a small repository change.",
  spark.title="Request-Aware Linear Flow",
  spark.description="A simple flow that accepts structured launch inputs.",
  spark.launch_inputs="[{\"key\":\"context.request.summary\",\"label\":\"Request Summary\",\"type\":\"string\",\"description\":\"Short description of the requested change.\",\"required\":true},{\"key\":\"context.request.target_paths\",\"label\":\"Target Paths\",\"type\":\"string[]\",\"description\":\"Optional files or directories the flow should focus on.\",\"required\":false},{\"key\":\"context.request.acceptance_criteria\",\"label\":\"Acceptance Criteria\",\"type\":\"string[]\",\"description\":\"Optional criteria the finished work should satisfy.\",\"required\":false}]"
];
```

This does two things:

- direct execution can render a launch form in the UI
- launch-time values become first-class `context.*` state for the run

To make a node consume those inputs, mention them in the prompt and document the contract:

```dot
plan [
  shape="box",
  label="Plan",
  prompt="Inspect the repository and produce a concrete plan for the stated goal. When present, use context.request.summary, context.request.target_paths, and context.request.acceptance_criteria to scope the work.",
  spark.reads_context="[\"context.request.summary\",\"context.request.target_paths\",\"context.request.acceptance_criteria\"]"
];
```

On Codergen nodes, `spark.reads_context` is the deterministic prompt-input contract: Spark projects the declared live context keys into a dedicated prompt section and renders absent keys as `<missing>`. It still does not restrict generic runtime reads for non-LLM handlers.

When the development backend is running, launch the edited runtime flow through the source-checkout wrapper:

```bash
SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark run launch \
  --project "$PWD" \
  --flow my-first-flow.dot \
  --summary "Run the first-flow tutorial against this checkout." \
  --goal "Add a /healthz endpoint that returns {\"status\":\"ok\"}." \
  --launch-context-json '{"context.request.summary":"Add a health endpoint.","context.request.target_paths":["src"],"context.request.acceptance_criteria":["Expose /healthz","Add or update tests"]}'
```

If you are using the native Cargo-built server above, run the same request through the Cargo-built CLI binary:

```bash
SPARK_API_BASE_URL=http://127.0.0.1:8010 target/debug/spark run launch \
  --project "$PWD" \
  --flow my-first-flow.dot \
  --summary "Run the first-flow tutorial against this checkout." \
  --goal "Add a /healthz endpoint that returns {\"status\":\"ok\"}." \
  --launch-context-json '{"context.request.summary":"Add a health endpoint.","context.request.target_paths":["src"],"context.request.acceptance_criteria":["Expose /healthz","Add or update tests"]}'
```

Use `--launch-context-file` instead when the context is too large for a shell-safe inline JSON value. Launch context keys must be JSON object keys under `context.*`; the `spark.launch_inputs` schema controls the UI form and documents the values a flow expects.

## Part 3: Add A Real Review Loop

Once a flow needs iteration, split the work into explicit stages. Do not try to do planning, implementation, and review in one prompt.

The easiest way to learn this pattern is to start from the bundled review-loop example:

```bash
cp ~/.spark-dev/flows/examples/implement-review-loop.dot ~/.spark-dev/flows/my-review-loop.dot
uv run spark flow validate --file ~/.spark-dev/flows/my-review-loop.dot --text
```

The core shape is:

```dot
start -> plan -> implement -> review -> done
review -> implement [condition="outcome=fail"]
```

The important idea is that loops are driven by real outcomes, not by prose. A review node saying "needs work" in plain text is not enough. The flow must route on `outcome=fail`.

### Pass Feedback Forward

When one node should guide a later node, use `context_updates` and stable `context.*` keys.

The review starter in this repo uses:

- `context.review.summary`
- `context.review.required_changes`
- `context.review.blockers`

That contract is documented on the nodes themselves:

```dot
implement [
  shape="box",
  spark.reads_context="[\"context.request.summary\",\"context.request.target_paths\",\"context.request.acceptance_criteria\",\"context.review.summary\",\"context.review.required_changes\",\"context.review.blockers\"]"
];

review [
  shape="box",
  spark.reads_context="[\"context.request.summary\",\"context.request.target_paths\",\"context.request.acceptance_criteria\"]",
  spark.writes_context="[\"context.review.summary\",\"context.review.required_changes\",\"context.review.blockers\"]"
];
```

### Make Review Return A Real Outcome

For Codex-backed review nodes, the most reliable pattern is a strict JSON status envelope:

```json
{
  "outcome": "fail",
  "notes": "The change is close but missing regression coverage.",
  "failure_reason": "Add a test before landing.",
  "context_updates": {
    "context.review.summary": "Behavior looks correct, but coverage is missing.",
    "context.review.required_changes": "Add a regression test for the changed path.",
    "context.review.blockers": ""
  }
}
```

That lets Spark turn the review result into:

- a real `fail` outcome for routing
- structured context that the next implementation pass can use

When you set `codergen.response_contract="status_envelope"`, Spark appends the shared envelope schema automatically. It also derives node-specific `context_updates` guidance from `spark.writes_context`: review nodes with declared writes are shown the exact keys they may emit, and nodes without declared writes are told not to emit `context_updates`. Runtime enforcement still validates the result if the model ignores that guidance.

## Part 4: Add A Tool Node And Preserve Artifacts

Tool nodes are for shell commands. They use `shape=parallelogram` or `type="tool"`, and their command now lives under the `tool.*` namespace.

Example:

```dot
validate [
  shape="parallelogram",
  label="Validate",
  timeout="10m",
  tool.command="uv run pytest --json-report --json-report-file=report.json -q",
  tool.artifacts.paths="report.json",
  tool.artifacts.stdout="pytest.stdout.txt",
  tool.artifacts.stderr="pytest.stderr.txt"
];
```

This pattern is useful when a node produces files you want to keep with the run.

Artifact capture rules:

- `tool.artifacts.paths` is a comma-separated list of relative paths or globs
- `tool.artifacts.stdout` and `tool.artifacts.stderr` preserve captured streams as extra artifacts
- captured files show up in the run's artifact browser

Use `context.*` for small facts and use artifacts for larger outputs.

Good examples for artifacts:

- test reports
- coverage files
- generated JSON reports
- screenshots
- build outputs that matter to the run

## Part 5: Validate And Debug

Use this loop while authoring:

1. Edit the flow in the UI or in DOT.
2. Validate it.
3. Run it.
4. Inspect the run's checkpoint, context, and artifacts.
5. Tighten prompts or routing if behavior was wrong.

Validation command:

```bash
uv run spark flow validate --file ~/.spark-dev/flows/<name>.dot --text
```

When a run behaves unexpectedly, check these first:

- the stated goal is concrete enough
- conditioned edges match the outcomes you actually produce
- unconditional fallback edges exist where you expect them
- nodes that rely on prior feedback explicitly mention the relevant `context.*` keys
- review nodes return real status envelopes instead of only prose

## What To Learn Next

After this tutorial, the next useful flows are:

- [src/spark/flows/software-development/implement-change-request.dot](../src/spark/flows/software-development/implement-change-request.dot) for implementing an approved durable change request from `changes/<CR-id>/request.md`, with runtime state kept under `.spark/change-requests/<CR-id>/` and the active workspace carried through `context.change_request.*`
- [src/spark/flows/software-development/spec-implementation/implement-spec.dot](../src/spark/flows/software-development/spec-implementation/implement-spec.dot) for long-running software-development spec implementation with committed artifacts under `specs/<slug>/` and runtime state under `.spark/spec-implementation/<slug>/`
- [src/spark/flows/examples/human-review-loop.dot](../src/spark/flows/examples/human-review-loop.dot) for explicit human approval
- [src/spark/flows/examples/parallel-review.dot](../src/spark/flows/examples/parallel-review.dot) for fan-out and fan-in
- [src/spark/flows/examples/supervision/supervised-implementation.dot](../src/spark/flows/examples/supervision/supervised-implementation.dot) for parent/child supervision with `stack.manager_loop`

Use these documents for the next level of detail:

- [README.md](../README.md) for the product overview and flow-building heuristics
- [src/spark/guides/dot-authoring.md](../src/spark/guides/dot-authoring.md) for the full authored DOT surface
- [specs/attractor-spec.md](../specs/attractor-spec.md) for exact runtime semantics
