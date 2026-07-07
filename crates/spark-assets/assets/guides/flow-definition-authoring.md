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
