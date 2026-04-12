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

Install dependencies and seed the runtime tree:

```bash
uv sync --dev
npm --prefix frontend install
uv run spark-server init
```

Start the app:

```bash
just run
```

That gives you:

- the backend at `http://127.0.0.1:8000`
- the frontend at `http://127.0.0.1:5173`
- a local flow library at `~/.spark/flows`

## Part 1: Start From A Minimal Flow

Seeded starter flows already live in `~/.spark/flows`. Copy the smallest one so you can edit your own version:

```bash
cp ~/.spark/flows/simple-linear.dot ~/.spark/flows/my-first-flow.dot
spark-workspace validate-flow --flow my-first-flow.dot --text
```

Open `my-first-flow.dot` in the Editor.

The starter looks like this structurally:

```dot
start -> plan -> implement -> summarize -> done
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
spark-workspace validate-flow --flow my-first-flow.dot --text
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

## Part 3: Add A Real Review Loop

Once a flow needs iteration, split the work into explicit stages. Do not try to do planning, implementation, and review in one prompt.

The easiest way to learn this pattern is to start from the bundled review-loop starter:

```bash
cp ~/.spark/flows/implement-review-loop.dot ~/.spark/flows/my-review-loop.dot
spark-workspace validate-flow --flow my-review-loop.dot --text
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
spark-workspace validate-flow --flow <name> --text
```

When a run behaves unexpectedly, check these first:

- the stated goal is concrete enough
- conditioned edges match the outcomes you actually produce
- unconditional fallback edges exist where you expect them
- nodes that rely on prior feedback explicitly mention the relevant `context.*` keys
- review nodes return real status envelopes instead of only prose

## What To Learn Next

After this tutorial, the next useful flows are:

- [src/spark/starter_flows/implement-from-plan.dot](../src/spark/starter_flows/implement-from-plan.dot) for implementing an existing plan document from per-run `.spark/planflows/<run>/plan-source.md` and `.spark/planflows/<run>/state.json` snapshots, with `context.planflow.*` carrying the workspace paths through evaluation
- [src/spark/starter_flows/spec-implementation/implement-spec.dot](../src/spark/starter_flows/spec-implementation/implement-spec.dot) for long-running greenfield spec implementation with repo-local state under `.specflow/`
- [src/spark/starter_flows/human-review-loop.dot](../src/spark/starter_flows/human-review-loop.dot) for explicit human approval
- [src/spark/starter_flows/parallel-review.dot](../src/spark/starter_flows/parallel-review.dot) for fan-out and fan-in
- [src/spark/starter_flows/supervision/supervised-implementation.dot](../src/spark/starter_flows/supervision/supervised-implementation.dot) for parent/child supervision with `stack.manager_loop`

Use these documents for the next level of detail:

- [README.md](../README.md) for the product overview and flow-building heuristics
- [src/spark/guides/dot-authoring.md](../src/spark/guides/dot-authoring.md) for the full authored DOT surface
- [specs/attractor-spec.md](../specs/attractor-spec.md) for exact runtime semantics
