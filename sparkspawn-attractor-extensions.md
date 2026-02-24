# Sparkspawn Attractor Extensions

This document defines **Sparkspawn-specific extensions** that are not part of the core `attractor-spec.md`.
Extensions are intended for UI/UX features and convenience defaults that the engine **must ignore** unless
explicitly implemented. The goal is to keep UI behavior deterministic while preserving core spec compliance.

---

## 1. UI Default LLM Selection (Graph-Level UI Extension)

### 1.1 Overview

The editor may store **per-flow UI defaults** for LLM selection. These defaults are used when creating
new nodes and for initializing UI fields. They are **UI-only** and **do not affect runtime execution**
unless the UI writes explicit node attributes.

### 1.2 Storage (Graph Attributes)

These attributes live in the DOT `graph [ ... ]` block and are persisted with the flow:

| Key                      | Type   | Default | Description |
|--------------------------|--------|---------|-------------|
| `ui_default_llm_model`   | String | `""`    | Default model ID shown/seeded by the UI. |
| `ui_default_llm_provider`| String | `""`    | Default provider key shown/seeded by the UI. |
| `ui_default_reasoning_effort` | String | `""` | Default reasoning effort shown/seeded by the UI. |

### 1.3 Behavior

When a new node is created in the UI:

1. The editor **may prefill** node LLM fields using the `ui_default_*` values.
2. The editor **may persist** these values into the node’s explicit attributes when saving.
3. The engine **must ignore** `ui_default_*` attributes; they are UI-only metadata.

### 1.4 Interaction With Core Spec

If the UI writes explicit node attributes (`llm_model`, `llm_provider`, `reasoning_effort`),
those values **override** `model_stylesheet` rules per the core spec precedence order.

### 1.5 Example

```
digraph Example {
    graph [
        ui_default_llm_model="gpt-5.2",
        ui_default_llm_provider="openai",
        ui_default_reasoning_effort="high"
    ];

    start [shape=Mdiamond];
    task  [label="Draft plan"];
    exit  [shape=Msquare];

    start -> task -> exit;
}
```
