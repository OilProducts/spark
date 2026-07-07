# Attractor FlowDefinition Specification

Attractor workflows are authored as typed YAML `FlowDefinition` documents. The Rust
`FlowDefinition` structs and generated JSON Schema are the public validation contract
for first-party Spark behavior.

## Authored Format

A flow document contains:

- `schema_version`, `id`, `title`, `description`, and `goal` flow metadata.
- `inputs` for launch-time context requirements.
- `defaults` for execution defaults such as model, provider, reasoning effort,
  fidelity, and retry limits.
- `nodes`, keyed by node id, where each node declares a typed `kind`.
- `edges`, each with `from`, `to`, optional `label`, `condition`, `weight`, and
  transition metadata.

Core runtime behavior is represented by typed fields:

- `config` stores node-specific behavior such as `AgentTask.prompt`,
  `Parallel.join_policy`, `Parallel.max_parallel`, `Parallel.join_k`,
  `Parallel.join_quorum`, and `Subflow.flow_ref`.
- `runtime` stores execution flags such as `allow_partial`, `auto_status`,
  `goal_gate`, `error_policy`, `fidelity`, and retry targets.
- `contracts` stores context read/write declarations and response contracts.
- `manager` stores manager-loop settings.
- `retry`, `execution`, and `ui` store retry policy, model selection, and layout
  metadata.

`extensions` remain available for product-specific metadata, but first-party runtime
semantics must not depend on duplicated extension keys when a typed field exists.

## Runtime Semantics

The runtime resolves handlers from `FlowNode.kind`. Built-in node kinds are:

- `start`
- `exit`
- `agent_task`
- `human_gate`
- `conditional`
- `parallel`
- `fan_in`
- `tool`
- `subflow`

Every executable flow must contain exactly one `start` node and at least one `exit`
node. Edges define traversal order and route conditions. Conditions evaluate against
observable runtime context and node outcomes.

Context read/write authority is enforced from `NodeContracts`. Runtime-generated
namespaces such as `_attractor.*` are reserved for the engine unless a typed runtime
path explicitly owns the write.

## Preview and Validation

Preview payloads expose the typed flow, typed nodes, typed edges, diagnostics, and
child previews. The canonical frontend model is the `FlowDefinition` payload rather
than a graph-source string.

Validation is performed by `FlowDefinition::validate()` and the generated JSON Schema
under `crates/spark-assets/assets/schemas/flow-definition.schema.json`.

## Example

```yaml
schema_version: "1.0"
id: simple_linear
title: Simple Linear Workflow
goal: Inspect the repository, make one targeted improvement, and summarize the result.
defaults:
  llm_provider: codex
  llm_model: gpt-5.5
nodes:
  start:
    kind: start
    config:
      kind: start
  plan:
    kind: agent_task
    label: Plan
    config:
      kind: agent_task
      prompt: Inspect the repository and plan the targeted improvement.
    runtime:
      error_policy: continue
    contracts:
      writes_context:
        - context.plan
  done:
    kind: exit
    config:
      kind: exit
edges:
  - from: start
    to: plan
  - from: plan
    to: done
```
