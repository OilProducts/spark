# Sparkspawn Attractor Extensions

This document defines **Sparkspawn-specific extensions** that are not part of the core `attractor-spec.md`.
Extensions are intended for UI/UX features and convenience defaults that the engine **must ignore** unless
explicitly implemented. The goal is to keep UI behavior deterministic while preserving core spec compliance.

---

## 1. UI Default LLM Selection (Global + Flow)

### 1.1 Overview

The editor stores **global UI defaults** and **optional per-flow overrides** for LLM selection.
Defaults are used when creating new nodes and for initializing UI fields. They are **UI-only**
and **do not affect runtime execution** unless the UI writes explicit node attributes.

### 1.2 Storage (Global Defaults)

Global defaults are stored in the UI layer (outside DOT). A local settings store is acceptable
(for example, browser local storage) as long as it is deterministic and user-editable.

Recommended key (client-side):

- `sparkspawn.ui_defaults`

### 1.3 Storage (Graph Attributes)

These attributes live in the DOT `graph [ ... ]` block and are persisted with the flow:

| Key                      | Type   | Default | Description |
|--------------------------|--------|---------|-------------|
| `ui_default_llm_model`   | String | `""`    | Flow-level default model ID shown/seeded by the UI. |
| `ui_default_llm_provider`| String | `""`    | Flow-level default provider key shown/seeded by the UI. |
| `ui_default_reasoning_effort` | String | `""` | Flow-level default reasoning effort shown/seeded by the UI. |

### 1.4 Behavior

When a new node is created in the UI:

1. If flow-level `ui_default_*` values are present, the editor **prefers** them.
2. Otherwise, the editor **falls back** to the global defaults.
3. The editor **may prefill** node LLM fields using the resolved defaults.
4. The editor **may persist** these values into the node’s explicit attributes when saving.
5. The engine **must ignore** `ui_default_*` attributes; they are UI-only metadata.

### 1.5 Interaction With Core Spec

If the UI writes explicit node attributes (`llm_model`, `llm_provider`, `reasoning_effort`),
those values **override** `model_stylesheet` rules per the core spec precedence order.

### 1.6 Example

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
