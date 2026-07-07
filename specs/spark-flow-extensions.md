# Spark Flow Extensions

Spark workflows use the typed Attractor `FlowDefinition` YAML model described in
[attractor-spec.md](attractor-spec.md). This document records Spark-owned metadata
that may appear alongside the typed contract.

## Runtime Boundary

First-party runtime behavior is represented by typed fields:

- flow launch inputs use `inputs`
- model defaults use `defaults`
- node prompts and node-specific behavior use `config`
- execution flags use `runtime`
- context read/write and response contracts use `contracts`
- manager-loop settings use `manager`
- retry and model overrides use `retry` and `execution`

Spark extensions must not be required duplicates for those typed fields. If a typed
field exists, runtime and preview code read that field.

## Extension Classes

Spark-specific `extensions` keys are allowed for product metadata that does not fit
the core contract:

- `Persisted but non-semantic metadata`: display hints or product-only annotations.
- `Runtime-interpreted product metadata`: settings that Spark explicitly implements
  outside the core Attractor contract.
- `UI-only local state`: editor state that is not stored in the YAML file.

Unknown Spark extension keys should be preserved when possible and ignored by
first-party runtime semantics unless explicitly implemented.

## Current Metadata

Spark may store result-selection metadata under flow `extensions`:

- `spark.result_node`: optional node id whose response artifact is materialized as
  the run result.
- `spark.result_summary_enabled`: optional boolean enabling result summarization.
- `spark.result_summary_prompt`: optional summarizer prompt.

These keys do not alter node routing or handler dispatch.

Spark UI surfaces may also store non-semantic product annotations under `metadata`
or `extensions`. Such keys are not a replacement for typed `FlowDefinition` fields.

## Example

```yaml
schema_version: "1.0"
id: plan_generation
title: Plan Generation
description: Generate an execution plan from approved workspace context.
inputs:
  - key: context.request.summary
    label: Request Summary
    type: string
    description: Short launch summary.
    required: true
defaults:
  llm_provider: openai
  llm_model: gpt-5.2
  reasoning_effort: high
nodes:
  start:
    kind: start
    config:
      kind: start
  task:
    kind: agent_task
    label: Draft plan
    config:
      kind: agent_task
      prompt: Draft a plan from context.request.summary.
    contracts:
      reads_context:
        - context.request.summary
      writes_context:
        - context.plan.summary
  exit:
    kind: exit
    config:
      kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: exit
extensions:
  spark.result_node: task
```
