# Spark Flow Extensions

This document defines Spark-owned flow-surface extensions layered onto [attractor-spec.md](attractor-spec.md).

It consolidates and supersedes `spark-attractor-extensions.md` as the canonical contract for Spark-specific flow metadata and authoring extensions.

## 1. Purpose

Spark may attach product-specific metadata to the flow surface defined by Attractor.

These extensions:
- may be persisted in DOT
- may also exist as local client defaults outside DOT
- do not alter Attractor core semantics unless a host product explicitly interprets them

## 2. Relationship to `attractor-spec.md`

Attractor remains authoritative for:
- DOT syntax
- execution semantics
- handler behavior
- model stylesheet semantics
- validation rules that are not explicitly extended here

This document defines additions and interpretation rules for Spark only. It does not modify the Attractor grammar or introduce new core handler types.

## 3. Extension Classification Model

Spark flow-surface extensions fall into three classes:

1. `UI-only local state`
   - not stored in DOT
   - not part of flow interchange

2. `Persisted but non-semantic flow metadata`
   - stored in DOT
   - used by Spark authoring or presentation
   - ignored by Attractor execution semantics

3. `Runtime-interpreted product extensions`
   - stored in DOT
   - interpreted by Spark runtime behavior if explicitly implemented

Every Spark extension key should declare which class it belongs to.

## 4. UI-Only Local Defaults

Global editor defaults may exist outside DOT. Their purpose is to seed authoring choices, not create runtime semantics.

Example client-side key:
- `spark.ui_defaults`

Storage details for local defaults are implementation examples, not an interoperability contract.

## 5. Persisted Flow-Level Metadata

The following graph attributes are persisted Spark metadata:

| Key | Type | Class | Default | Meaning |
| --- | --- | --- | --- | --- |
| `spark.title` | String | `Persisted but non-semantic flow metadata` | `""` | Spark display title for flow discovery and authoring surfaces. Falls back to graph `label` when unset. |
| `spark.description` | String | `Persisted but non-semantic flow metadata` | `""` | Spark short description for flow discovery and authoring surfaces. Falls back to graph `goal` when unset. |
| `spark.launch_inputs` | JSON-encoded array string | `Runtime-interpreted product extension` | `""` | Spark launch-form schema. The product may render a launch form from it and map the submitted values into Attractor `launch_context` under `context.*`. |
| `ui_default_llm_model` | String | `Product launch metadata` | `""` | Flow-level default model id shown or seeded by Spark authoring tools, and used as the launch-time model default when a run is started without an explicit model override. |
| `ui_default_llm_provider` | String | `Persisted but non-semantic flow metadata` | `""` | Flow-level default provider key shown or seeded by Spark authoring tools. |
| `ui_default_reasoning_effort` | String | `Persisted but non-semantic flow metadata` | `""` | Flow-level default reasoning-effort value shown or seeded by Spark authoring tools. |

These attributes live in the DOT `graph [ ... ]` block.

`spark.title` and `spark.description` are persisted but non-semantic metadata. `ui_default_llm_model` is launch metadata: Spark/Attractor may use it to choose the run model when the caller does not explicitly provide one. The remaining `ui_default_*` fields stay non-semantic unless a product surface chooses to interpret them.

`spark.launch_inputs` is product-interpreted metadata. It does not change Attractor execution semantics by itself, but Spark may use it to gather launch-time values and construct `launch_context` for a run.

### 5.1 `spark.launch_inputs` Shape

`spark.launch_inputs` stores a JSON array of objects with this shape:

```json
[
  {
    "key": "context.request.summary",
    "label": "Request Summary",
    "type": "string",
    "description": "Short description shown in the launch form.",
    "required": true
  }
]
```

Rules:
- `key` must be a `context.*` key
- `label` is the user-facing launch form label
- `type` must be one of `string`, `string[]`, `boolean`, `number`, or `json`
- `description` is optional explanatory text shown in Spark launch surfaces
- `required` controls whether Spark blocks launch when the value is omitted

Spark is responsible for validating this schema before using it. Attractor consumers that ignore Spark extensions may ignore this field entirely.

## 6. Persisted Node-Level Authoring Metadata

Spark may persist the following node attributes as authoring metadata:

| Key | Type | Class | Default | Meaning |
| --- | --- | --- | --- | --- |
| `spark.reads_context` | JSON-encoded array string | `Persisted but non-semantic flow metadata` | `""` | Declares which live context keys the node is expected to consume from launch state or earlier stages. For Codergen nodes, this is also the deterministic prompt-projection contract for declared inputs. |
| `spark.writes_context` | JSON-encoded array string | `Runtime-enforced authoring contract` | `""` | Declares which `context_updates` keys the node may emit. This is a runtime-enforced exact write allowlist after key normalization; a missing declaration means no node-authored writes are allowed. |

These attributes live on individual DOT nodes.

Rules:
- each declaration encodes a JSON array of strings
- each `spark.reads_context` declaration encodes exact live-context keys after runtime normalization; bare keys are allowed when intentionally declared
- each declared `spark.writes_context` string names an exact `context_updates` key after runtime normalization; bare keys are allowed when intentionally emitted
- `spark.reads_context` does not restrict generic runtime reads
- `spark.writes_context` is a runtime-enforced exact write contract
- a missing `spark.writes_context` declaration means the node may not emit node-authored `context_updates`

Spark may use these declarations for:
- node inspector affordances
- documentation and authoring hints
- launch and review UX
- future validation of flow authoring contracts

Attractor execution semantics change for both keys in different ways: the runtime enforces `spark.writes_context` as the node's exact normalized write allowlist, and Codergen uses `spark.reads_context` as the deterministic declared-input projection into the prompt. `spark.reads_context` still does not restrict generic runtime reads outside that Codergen prompt contract.

## 7. Authoring Behavior

When a new node or flow is created in Spark authoring tools:

1. On flow creation, the editor may snapshot global defaults into `ui_default_*`.
2. If a flow lacks `ui_default_*`, the editor may seed them once from the current global defaults.
3. New nodes use the flow snapshot as their default.
4. Graph-level launch input definitions may be persisted into `spark.launch_inputs`.
5. Node-level context read/write declarations may be persisted into `spark.reads_context` and `spark.writes_context`.
6. Global defaults do not retroactively update existing flows.
7. The editor may persist explicit node attributes when the operator chooses values that should become semantic runtime inputs.

These are Spark authoring rules, not Attractor requirements.

## 8. Interaction With Core Runtime Attributes

If Spark writes explicit node attributes such as:
- `llm_model`
- `llm_provider`
- `reasoning_effort`

those values are core Attractor attributes and follow Attractor precedence and runtime semantics.

`ui_default_*` metadata never overrides execution semantics by itself.

This is the critical distinction:
- `ui_default_*` is metadata
- explicit node attrs are semantic runtime inputs

This section also applies to the new Spark authoring-contract keys:
- `spark.launch_inputs` may cause Spark to construct Attractor `launch_context`, but it is not itself an Attractor runtime key
- `spark.reads_context` remains non-restrictive for generic runtime reads, but Codergen uses it as the guaranteed declared-input prompt surface
- `spark.writes_context` is a runtime-enforced exact allowlist for node-authored `context_updates`; codergen response-contract nodes use the same-thread repair loop when their emitted keys violate that contract

## 9. Workspace Launch Policy

Whether an agent may independently initiate a flow is not stored in DOT.

Launch policy is a workspace-global Spark setting, currently modeled outside the flow file in the workspace flow catalog. This keeps the flow file self-describing while preserving host-product control over exposure policy.

Current launch-policy values are:
- `agent_requestable`
- `trigger_only`
- `disabled`

This is intentionally separate from persisted DOT metadata:
- `spark.title` and `spark.description` describe the flow itself
- launch policy describes how the workspace exposes that flow in a given installation

## 10. Validation and Ignore Rules

Attractor implementations that do not understand Spark extension attributes may ignore them unless a host product explicitly adopts them.

Spark tooling may:
- preserve them
- surface them in authoring UIs
- validate them as Spark-owned metadata
- reject malformed `spark.launch_inputs`
- reject malformed `spark.reads_context` / `spark.writes_context`

Unknown Spark extension attributes should have explicit handling rules in Spark tooling rather than accidental or inconsistent behavior.

## 11. Compatibility and Forward-Compatibility Constraints

Flows containing only Spark metadata remain executable as standard Attractor flows because these keys are host-product extensions rather than Attractor core attrs.

Future Spark extension keys should declare:
- their classification
- whether they are DOT-persisted
- whether they are semantic or non-semantic

New keys should avoid accidental collision with future Attractor core attributes.

## 12. Examples

Example:

```dot
digraph Example {
    graph [
        spark.title="Plan Generation",
        spark.description="Generate an execution plan from approved workspace context.",
        spark.launch_inputs="[{\"key\":\"context.request.summary\",\"label\":\"Request Summary\",\"type\":\"string\",\"description\":\"Short launch summary.\",\"required\":true}]",
        ui_default_llm_model="gpt-5.2",
        ui_default_llm_provider="openai",
        ui_default_reasoning_effort="high"
    ];

    start [shape=Mdiamond];
    task  [
        label="Draft plan",
        spark.reads_context="[\"context.request.summary\"]",
        spark.writes_context="[\"context.plan.summary\"]"
    ];
    exit  [shape=Msquare];

    start -> task -> exit;
}
```

In this example:
- the `spark.title` and `spark.description` keys are persisted non-semantic discovery metadata
- the `spark.launch_inputs` key is Spark launch-form metadata that may be converted into Attractor `launch_context`
- the `spark.reads_context` and `spark.writes_context` keys are persisted node-level authoring metadata
- the `ui_default_*` keys are persisted non-semantic metadata
- runtime semantics do not change unless Spark later materializes explicit node attrs
- an Attractor-only consumer may parse the flow and ignore the Spark metadata
