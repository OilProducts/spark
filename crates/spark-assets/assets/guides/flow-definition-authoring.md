# Spark FlowDefinition YAML Authoring Guide

Spark workflows are authored as FlowDefinition YAML files. The Rust
`FlowDefinition` type is the source of truth, and
`assets/schemas/flow-definition.schema.json` is the public validation contract.

## Minimal Flow

```yaml
schema_version: "1"
id: simple_linear
title: Simple Linear Workflow
description: Plan, implement, and summarize a small change.
goal: Inspect the repository, make one targeted improvement, and summarize it.
nodes:
  start:
    kind: start
    label: Start
  plan:
    kind: agent_task
    label: Plan
    config:
      kind: agent_task
      prompt: Inspect the repository and plan the work.
  done:
    kind: exit
    label: Done
edges:
  - from: start
    to: plan
  - from: plan
    to: done
```

## Top-Level Fields

- `schema_version`: current value is `"1"`.
- `id`: stable flow id, using letters, digits, `_`, or `-`.
- `title`, `description`, and `goal`: catalog and runtime metadata.
- `inputs`: launch inputs with `key`, `label`, `type`, `description`, `required`,
  and optional `default`.
- `defaults`: common model, provider, reasoning, fidelity, and retry defaults.
- `nodes`: map of node id to typed node definition.
- `edges`: ordered transitions between nodes.
- `metadata` and `extensions`: compatibility and experimental metadata.

## Node Kinds

Supported node kinds are:

- `start`
- `exit`
- `agent_task`
- `human_gate`
- `conditional`
- `parallel`
- `fan_in`
- `tool`
- `subflow`

Use `config.kind` to select the matching tagged config variant when a node needs
configuration. For example, `agent_task` and `human_gate` use `prompt`, `tool`
uses `command`, and `subflow` uses `flow_ref` plus optional `input_map`.

## Tool Context Bindings

Tool nodes compose with context through two optional maps:

- `env_map` binds declared context values into named environment variables, so
  commands consume context without shell interpolation. Every binding must
  resolve to a non-null context value or the node fails before the command runs.
- `output_map` parses the command's stdout as JSON and maps dotted paths into
  declared context keys. Commands used with `output_map` must print only JSON
  on stdout (redirect noisy tooling to `/dev/null` or stderr). Mapped keys
  outside `context.tool.*` must be declared in `contracts.writes_context`.

```yaml
prepare_workspace:
  kind: tool
  config:
    kind: tool
    command: |
      git worktree add -b "spark/task/${SPARK_RUN_ID}" "../wt-${SPARK_RUN_ID}" >/dev/null
      printf '{"path":"../wt-%s"}' "$SPARK_RUN_ID"
    env_map:
      SPARK_RUN_ID: internal.run_id
    output_map:
      context.workspace.path: path
  contracts:
    writes_context:
      - context.workspace.path
```

## Run Result Summaries

An exit node may opt in to producing the run's result:

```yaml
done:
  kind: exit
  config:
    kind: exit
    result_summary: true
```

When the run reaches an opted-in exit — or fails, in which case any opted-in
exit counts — the runtime executes one summarizer agent whose working
directory is the run's artifact root. It reads the recorded transcripts
(`events.jsonl`, `checkpoint.json`, `logs/<node>/…`) directly and its markdown
response becomes the run result shown in the Runs tab. `result_summary_prompt`
overrides the default instructions. A summarizer failure never fails the run;
the result falls back to the exit predecessor's response with the error
recorded.

## Subflow Working Directories

A subflow's child run normally inherits the parent's working directory, or an
authored static `manager.child_workdir`. To bind the child to a directory
produced by an earlier node, use `manager.child_workdir_from` with a context
key. The runtime validates that the resolved directory exists and is either
inside the run's working directory or a linked git worktree of the same
repository. Declaring both `child_workdir` and `child_workdir_from` fails the
node. Run working directories are immutable once a run starts; crossing a
workspace boundary always means launching a child run bound to that workspace.

```yaml
implement:
  kind: subflow
  config:
    kind: subflow
    flow_ref: workers/implement-task.yaml
  manager:
    child_workdir_from: context.workspace.path
```

## Edges

Each edge has `from` and `to`. Optional fields are `label`, `condition`,
`weight`, `transition`, and `extensions`.

Conditions use the runtime expression subset, such as:

```yaml
condition: outcome=success
condition: outcome=fail && preferred_label=Rewrite
```

## Validation

Validate authored files before saving or launching:

```bash
spark flow validate --file crates/spark-assets/assets/flows/examples/simple-linear.yaml --text
```

Validation parses YAML into `FlowDefinition`, checks the generated schema shape,
applies semantic rules, and then verifies the runtime graph derived from the
typed model.
