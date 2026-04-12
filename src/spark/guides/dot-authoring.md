# Spark DOT Authoring Guide

This guide is for agents editing `.dot` workflow files directly.

It is intentionally more operational than the specs. For exact runtime semantics, Attractor remains authoritative. This guide summarizes the documented authoring surface that Spark expects and exposes.

## Boundary

- The active project repository is the source of truth for domain work and codebase context.
- The flow library is the user-owned directory of editable `.dot` workflows.
- Do not edit conversation state, project metadata, or other files under `~/.spark` as part of normal flow authoring.

## Validation Loop

After changing a flow, validate it before claiming the edit is finished:

```bash
spark flow validate --file /path/to/flow.dot --text
```

Use `--file` when you are editing a `.dot` file directly. Use `--flow <name>` only when you want to validate a flow that already exists in Spark's runtime flow library.

## Structural Building Blocks

Attractor flows are Graphviz `digraph` files.

```dot
digraph my_flow {
  graph [
    label="Human-readable title",
    goal="Stated goal for the run",
    spark.title="Catalog title",
    spark.description="Catalog description"
  ];

  start [shape=Mdiamond];
  plan [shape=box, prompt="Inspect the repository and plan the work."];
  done [shape=Msquare];

  start -> plan;
  plan -> done;
}
```

The main authorable structures are:

- graph attributes in `graph [ ... ]`
- nodes declared as `node_id [ ... ]`
- edges declared as `from -> to [ ... ]`
- optional `node [ ... ]` and `edge [ ... ]` default blocks
- optional subgraphs for scoped defaults and stylesheet classes

Chained edges are legal:

```dot
start -> plan -> implement -> done
```

That expands to one edge per hop.

## Value Types

Common documented value types:

- strings: `"hello"`
- integers: `0`, `3`, `-1`
- floats: `0.5`, `-3.14`
- booleans: `true`, `false`
- durations: `250ms`, `45s`, `15m`, `2h`, `1d`

The parser also accepts documented bare values where the grammar allows them, such as model ids like `gpt-5`.

## Nodes vs Edges

Important distinction:

- nodes have handler types and prompts
- edges do not have types
- edge behavior comes from attributes such as `condition`, `label`, `weight`, `fidelity`, `thread_id`, and `loop_restart`

## Node Types And Shape Mapping

The documented node surface is:

| Shape | Default type | Meaning |
| --- | --- | --- |
| `Mdiamond` | `start` | Entry node |
| `Msquare` | `exit` | Terminal node |
| `box` | `codergen` | Normal LLM-backed stage |
| `hexagon` | `wait.human` | Human gate |
| `diamond` | `conditional` | Conditional pass-through node |
| `component` | `parallel` | Parallel fan-out |
| `tripleoctagon` | `parallel.fan_in` | Parallel fan-in |
| `parallelogram` | `tool` | Tool execution |
| `house` | `stack.manager_loop` | Parent/child supervisor |

You can also set `type="..."` explicitly. That overrides shape-based resolution.

Documented handler types:

- `start`
- `exit`
- `codergen`
- `wait.human`
- `conditional`
- `parallel`
- `parallel.fan_in`
- `tool`
- `stack.manager_loop`

Prefer explicit `type=` when the behavior matters more than the rendered shape.

## Core Graph Attributes

Documented Attractor graph attributes:

| Key | Type | Meaning |
| --- | --- | --- |
| `goal` | string | Stated goal for the run. Exposed to prompts as `$goal`. |
| `label` | string | Human-readable graph label. |
| `model_stylesheet` | string | CSS-like LLM defaults. |
| `default_max_retries` | integer | Default additional retries for nodes without `max_retries`. |
| `retry_target` | string | Graph-level retry jump target when exiting with unmet goal gates. |
| `fallback_retry_target` | string | Secondary graph-level retry target. |
| `default_fidelity` | string | Default context fidelity mode. |
| `stack.child_dotfile` | string | Child flow path for manager-loop supervision. |
| `stack.child_workdir` | string | Child run working directory. |
| `tool.hooks.pre` | string | Shell command before tool execution. |
| `tool.hooks.post` | string | Shell command after tool execution. |

Notes:

- `default_max_retries` is the canonical key. `default_max_retry` is legacy input only and should not be authored.
- `stack.child_workdir` default `cwd` means the current pipeline run working directory.
- Relative `stack.child_dotfile` resolves from the parent flow source directory when the parent flow path is known.

## Core Node Attributes

Documented Attractor node attributes:

| Key | Type | Meaning |
| --- | --- | --- |
| `label` | string | Display label; defaults to the node id. |
| `shape` | string | Graphviz shape; determines the default handler. |
| `type` | string | Explicit handler type override. |
| `prompt` | string | Primary task instruction. Supports `$goal`. |
| `max_retries` | integer | Additional retries for the node. |
| `goal_gate` | boolean | Node must reach `SUCCESS` or `PARTIAL_SUCCESS` before pipeline exit. |
| `retry_target` | string | Node-level retry jump target. |
| `fallback_retry_target` | string | Secondary node-level retry target. |
| `fidelity` | string | Node-level context fidelity override. |
| `thread_id` | string | Explicit thread/session reuse key. |
| `class` | string | Comma-separated stylesheet classes. |
| `timeout` | duration | Max node execution time. |
| `llm_model` | string | Explicit model override. |
| `llm_provider` | string | Explicit provider override. |
| `reasoning_effort` | string | `low`, `medium`, or `high`. |
| `codergen.response_contract` | string | Codergen-only shared response-format contract. |
| `codergen.contract_repair_attempts` | integer | Codergen-only bounded same-thread repair budget for malformed response-contract output and structured write-contract violations. |
| `auto_status` | boolean | Auto-generate success when the handler writes no status. |
| `allow_partial` | boolean | Accept `PARTIAL_SUCCESS` when retries exhaust. |

## Handler-Specific Node Attributes

These attrs are only meaningful for specific node types:

### `codergen`

| Key | Type | Meaning |
| --- | --- | --- |
| `codergen.response_contract` | string | Shared response-format appendix injected by the runtime. Use `status_envelope` when the node must return only Attractor's structured Outcome JSON. |
| `codergen.contract_repair_attempts` | integer | Same-thread repair attempts for malformed response-contract output and structured write-contract violations. Defaults to `1` when a response contract is set, otherwise `0`. This is not the same as node `max_retries`, which reruns the whole node. |

Authoring guidance:

- Use `codergen.response_contract="status_envelope"` on codergen nodes that are expected to emit structured routing/context outcomes.
- Use `codergen.contract_repair_attempts` only when you need to override the default repair budget. The runtime already defaults to one repair attempt for response-contract nodes.
- Keep the node prompt focused on the task and any node-specific fields it must set; the runtime supplies the canonical envelope schema.
- Do not set this attr on `tool`, `wait.human`, `stack.manager_loop`, `start`, or `exit` nodes.

### `tool`

| Key | Type | Meaning |
| --- | --- | --- |
| `tool.command` | string | Shell command to run. |
| `tool.hooks.pre` | string | Node-level pre-hook override. |
| `tool.hooks.post` | string | Node-level post-hook override. |
| `tool.artifacts.paths` | string | Comma-separated relative paths/globs copied into run artifacts after execution. |
| `tool.artifacts.stdout` | string | Relative artifact path for captured stdout. |
| `tool.artifacts.stderr` | string | Relative artifact path for captured stderr. |

Artifact capture rules:

- `tool.artifacts.paths` is a comma-separated list, not JSON.
- Patterns are resolved relative to the tool execution working directory.
- Captured files are copied under `artifacts/<node_id>/captured/...` for the run.
- `tool.artifacts.stdout` and `tool.artifacts.stderr` write extra artifact files under `artifacts/<node_id>/...`.
- Use `context.*` for small semantic facts and artifact attrs for larger durable outputs.

### `parallel`

| Key | Type | Meaning |
| --- | --- | --- |
| `join_policy` | string | Join rule for branch completion. Documented values: `wait_all`, `first_success`. |
| `max_parallel` | integer | Max concurrent branches. |

`error_policy` exists in the current implementation but is not documented in the spec, so do not rely on it in authored flows unless and until it is promoted into the spec.

### `wait.human`

| Key | Type | Meaning |
| --- | --- | --- |
| `human.default_choice` | string | Default selected choice on timeout. |

### `stack.manager_loop`

| Key | Type | Meaning |
| --- | --- | --- |
| `manager.poll_interval` | duration | Poll cadence for observing child progress. |
| `manager.max_cycles` | integer | Max supervisor cycles. |
| `manager.stop_condition` | string | Condition expression evaluated against context. |
| `manager.actions` | string | Comma-separated manager actions such as `observe,wait` or `observe,steer,wait`. |
| `stack.child_autostart` | boolean | Whether the child pipeline should be started automatically. |

`manager.steer_cooldown` exists in the runtime implementation but is not currently documented in the spec, so it is intentionally omitted from this guide.

`context.stack.child.*` is runtime-owned manager-loop telemetry. Read it from authored flows if you need child status or outcome, but do not clear or set it from prompts or context updates as business logic.

With `stack.child_autostart=true`, each manager invocation starts a fresh child unless `context.stack.child.status` is already `running`. Terminal child state from an earlier invocation is treated as stale runtime snapshot data, not reusable authored state.

## Edge Attributes

Edges do not have types. The documented edge surface is:

| Key | Type | Meaning |
| --- | --- | --- |
| `label` | string | Human-facing caption and preferred-label routing key. |
| `condition` | string | Boolean routing expression. |
| `weight` | integer | Priority among equally eligible unconditional edges. |
| `fidelity` | string | Incoming-edge fidelity override for the target node. |
| `thread_id` | string | Incoming-edge thread/session override for the target node. |
| `loop_restart` | boolean | Restart the run with a fresh log directory instead of continuing in place. |

## Routing Behavior

Routing matters when authoring edges:

1. The engine evaluates conditioned edges first.
2. If no condition matches, it checks unconditional edges for `preferred_label`.
3. Then unconditional edges for suggested next ids.
4. Then unconditional edges by highest `weight`.
5. Then unconditional edges by lexical target-node order.
6. If no eligible edge remains after a non-failing outcome, the pipeline completes successfully.

A false conditioned edge is not a fallback.

## Condition Language

Documented condition variables:

- `outcome`
- `preferred_label`
- any `context.*` key

Documented operators:

- `=`
- `!=`
- `&&`

Examples:

```dot
review -> done      [condition="outcome=success"]
review -> implement [condition="outcome=fail"]
gate -> revise      [condition="preferred_label=Revise"]
validate -> done    [condition="context.request.approved=true && outcome=success"]
```

## Fidelity And Threading

Documented fidelity values:

- `full`
- `truncate`
- `compact`
- `summary:low`
- `summary:medium`
- `summary:high`

Resolution order:

1. incoming edge `fidelity`
2. target node `fidelity`
3. graph `default_fidelity`
4. runtime fallback

Thread/session resolution for `full` fidelity follows the same pattern:

1. target node `thread_id`
2. incoming edge `thread_id`
3. derived runtime thread key

## Model Stylesheet

`model_stylesheet` lets you define default `llm_model`, `llm_provider`, and `reasoning_effort` centrally.

Documented selectors:

- `*`
- shape selectors like `box` or `house`
- `.class`
- `#node_id`

Selector precedence:

1. `#node_id`
2. `.class`
3. shape selector
4. `*`

Allowed properties:

- `llm_model`
- `llm_provider`
- `reasoning_effort`

Example:

```dot
graph [
  model_stylesheet="
    * { llm_provider: openai; reasoning_effort: medium; }
    box { llm_model: gpt-5; }
    .review { reasoning_effort: high; }
    #finalize { llm_model: gpt-5.4; }
  "
];
```

## Subgraphs And Defaults

Subgraphs are authorable and useful.

Use them for:

- scoped `node [ ... ]` defaults
- scoped `edge [ ... ]` defaults
- grouping nodes visually
- adding stylesheet classes via subgraph structure

Example:

```dot
subgraph cluster_review {
  label = "Review Loop";
  node [thread_id="review-loop", timeout="900s"];

  implement [shape=box];
  review [shape=box, class="review"];
}
```

## Spark Graph Metadata

Spark stores additional graph-level authoring metadata in DOT:

| Key | Type | Meaning |
| --- | --- | --- |
| `spark.title` | string | Catalog/display title. |
| `spark.description` | string | Catalog/display description. |
| `spark.launch_inputs` | JSON-encoded array string | Launch-form schema used by Spark to build `launch_context`. |
| `ui_default_llm_model` | string | Flow-level UI default model. |
| `ui_default_llm_provider` | string | Flow-level UI default provider. |
| `ui_default_reasoning_effort` | string | Flow-level UI default reasoning effort. |

These are Spark-owned metadata, not Attractor core semantics.

### `spark.launch_inputs`

`spark.launch_inputs` is a JSON array encoded as a string in DOT.

Each entry has this documented shape:

```json
[
  {
    "key": "context.request.summary",
    "label": "Request Summary",
    "type": "string",
    "description": "Short launch summary.",
    "required": true
  }
]
```

Documented `type` values:

- `string`
- `string[]`
- `boolean`
- `number`
- `json`

The `key` must use the `context.*` namespace.

## Spark Node Metadata

Spark also stores node-level authoring metadata in DOT:

| Key | Type | Meaning |
| --- | --- | --- |
| `spark.reads_context` | JSON-encoded array string | Declared live context keys the node is expected to consume from launch state or earlier stages. Codergen projects these declared keys into a deterministic prompt section; generic runtime reads remain unrestricted. |
| `spark.writes_context` | JSON-encoded array string | Declared `context_updates` keys the node may emit. The runtime enforces this as the node's exact normalized write allowlist. |

These keys document intent for authors and the editor. `spark.reads_context` now also defines the deterministic declared-input prompt surface for Codergen nodes: Spark projects the declared keys in sorted order and renders absent keys as `<missing>`. It still does not restrict generic runtime reads for non-LLM handlers. `spark.writes_context` is runtime-enforced: a missing declaration means the node may not emit node-authored `context_updates`, comparison happens after runtime key normalization, and codergen response-contract nodes use same-thread repair when emitted keys violate the declaration.

## Context Passing Pattern

Use stable `context.*` keys for feedback that later nodes need.

Patterns:

- launch state arrives through `launch_context`
- handlers can emit `context_updates`
- later nodes see that state through shared run context and carryover
- edge conditions may branch on `context.*`

Example review response contract:

```json
{
  "outcome": "fail",
  "notes": "The draft is missing the acceptance criteria.",
  "failure_reason": "Revise before completion.",
  "context_updates": {
    "context.review.summary": "The work is close, but it does not satisfy the stated acceptance criteria.",
    "context.review.required_changes": "Address the missing acceptance criteria and rerun the review.",
    "context.review.blockers": ""
  }
}
```

Use domain-specific keys when software-review names are not appropriate.

Examples:

- `context.email.required_revisions`
- `context.research.open_questions`
- `context.request.target_paths`

## Parent/Child Flows

For `stack.manager_loop`:

- use relative `stack.child_dotfile` when bundling parent and child flows together
- relative child DOT paths resolve from the parent flow source directory
- child execution defaults to the parent run working directory unless `stack.child_workdir` is explicitly set
- avoid absolute `stack.child_workdir` values in portable shipped flows

## What This Guide Does Not Standardize

The parser and editor may preserve unknown attributes, but this guide only lists the documented flow surface.

Treat any extra attrs you encounter as one of:

- Spark metadata explicitly documented elsewhere
- implementation-specific runtime behavior not yet promoted into the spec
- project-specific extension attrs that are only meaningful to a custom host

Do not assume an undocumented attribute is portable just because it round-trips.

## Starter Flow Starting Points

- `simple-linear.dot`: smallest useful single-pass pattern
- `implement-review-loop.dot`: review-driven retry loop
- `human-review-loop.dot`: human approval / revise loop
- `parallel-review.dot`: fan-out / fan-in flow
- `supervision/supervised-implementation.dot`: parent/child composition

## Authoring Checklist

Before you finish a flow edit:

1. Ensure the graph has a valid start node and terminal path.
2. Confirm graph metadata uses `spark.title` and `spark.description` when the flow should be catalog-friendly.
3. Keep launch expectations in `spark.launch_inputs` when direct execution should render a form.
4. Keep node read/write context contracts in DOT metadata when they matter to future editors.
5. Validate with `spark flow validate --file /path/to/flow.dot --text`.
