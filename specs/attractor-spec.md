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

## Durable Recovery and Explicit Retry

Ordinary crash recovery reuses a complete, valid durable node outcome for the
checkpointed execution identity. Missing, malformed, or contract-invalid artifacts
remain unaccepted. `runtime.recovery_policy: pause` requires an explicit decision
before executing work without an accepted response; human gates retain their
journaled questions and answers.

Explicit Retry starts a fresh attempt under the same run ID and restores that
node's configured automatic-retry allowance. The reserved checkpoint context map
`internal.execution_attempt_bases` defaults to zero for older checkpoints. Artifact
attempts are the node's execution base plus its automatic retry count; backoff and
retry limits use only the automatic count. Preparation advances the target base
past its previous attempt and resets only that node's count. Prior artifacts,
logs, snapshots, working directories, and lineage remain intact.

`internal.retry_request` durably records a unique request ID, the target run, node,
stage, and linked child targets. The adjusted checkpoint is written before child
preparation or execution. Its `preparing` flag permits restart to finish interrupted
preparation idempotently. An applied request never advances attempts or resets
allowances again, and never authorizes a later node invocation. Recovery after a
prepared attempt's response reuses that response; it does not fall back to an older
attempt. Earlier completed visits to the same node do not complete the prepared
invocation: resume matches its run/node/stage while preserving completion history
and artifact stage identity. The legacy run-ID-only marker grants no execution
authorization.

Parent Retry validates the linked child's parent/node/root lineage, checkpoint,
and captured flow before acceptance, recursively through nested failed managers.
It resumes those children in place through the existing manager path, taking
precedence over historical-invocation replacement. Already-successful children are
consumed without execution or siblings. A child that fails again propagates its
new failure without repeating the request. Missing, ambiguous, or inconsistent
lineage is rejected. Custom launchers retain responsibility for their children;
retry requiring child execution is rejected for a custom launcher.
A directly retried child with its own request resumes after restart even while its
parent remains failed. Recovery validates its lineage and saved state and reserves
execution ownership for the tree before finishing preparation or resuming it.
Before acceptance, each child along its ancestor lineage must have a unique
parent/node/invocation index; duplicates return `retry_ambiguous_child_invocation`
without changing checkpoints or counters. A unique legacy child with no invocation
index remains supported.
Inherited requests stay with the manager; custom launchers retain recovery responsibility.

The existing executor ownership registry rejects overlapping retries in the run
tree. Cancellation and human waiting are preserved. A retry request authorizes only
its prepared invocation across restarts; unrelated recovery pauses still require
an explicit decision. Retry returns the same run ID, Continue creates a new run
with fresh execution metadata, and `started` means execution was accepted rather
than successful.

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
