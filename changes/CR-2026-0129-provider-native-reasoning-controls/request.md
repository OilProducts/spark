# Provider-native reasoning controls (phase 2)

## Summary

Phase 1 (CR-2026-0127) made reasoning effort per model and provider-native; CR-2026-0128 made the model list live. Phase 2 adds the remaining reasoning controls providers actually offer, stores them alongside effort, sends each in the provider's own field, and shows each control only for models that offer it:

- thinking mode (adaptive, off, or a fixed token budget);
- OpenAI reasoning mode (`standard` or `pro`);
- reasoning summaries.

It also lets flows use any effort level a model offers.

## Stored settings

Extend `ModelSettings` (`crates/spark-common/src/settings.rs`) and its TypeScript counterpart (`ModelSettings` in `frontend/src/lib/api/settingsApi.ts`) with four optional fields. `null` means the provider's default, as `reasoning_effort` does today:

| Field | Values |
|---|---|
| `thinking` | `adaptive`, `off`, `budget` |
| `thinking_budget_tokens` | integer of at least 1024; required when `thinking` is `budget`, otherwise null |
| `reasoning_mode` | `standard`, `pro` |
| `reasoning_summary` | `auto`, `concise`, `detailed` |

- Validate these values where settings are saved (workspace, project, conversation, and mission model settings). Reject other values with a message naming the field.
- Documents written before this change load unchanged (the new fields default to null).
- A value the selected model does not offer is sent unchanged and the provider's error is surfaced, as for effort today.

## Capabilities

Report per model, in `/workspace/api/projects/chat-models`, which of these controls it offers, from the same sources and in the same order as CR-2026-0128: the provider's own data, then Spark's per-model notes, never adding a model to the list.

- **Anthropic API.** From `GET /v1/models` `capabilities.thinking.types`: `adaptive` when `adaptive.supported`, `budget` when `enabled.supported`. The capability tree does not say whether thinking can be turned off, and some models reject it (Opus 5.5 at every effort, Opus 5 above `high`). Check the tree for such an indicator first; if there is none, offer `off` only for models Spark's notes list as allowing it.
- **OpenAI API.** `reasoning_mode` (`standard`/`pro`, on GPT-5.6 and GPT-6 per OpenAI's reasoning guide) and `reasoning_summary` (`auto`/`concise`/`detailed`, on select models) from Spark's notes, since OpenAI's Model object reports no capabilities.
- **OpenRouter.** From each model's `reasoning` object: `off` unless `mandatory`, and summaries or budgets only where OpenRouter documents them for the model.
- **Codex.** If the app-server accepts a per-turn reasoning summary setting (the Codex CLI already supports `model_reasoning_summary` in its configuration), offer `reasoning_summary`; otherwise leave Codex unchanged. Verify against the installed app-server schema (`codex app-server generate-json-schema`).
- **Claude Code, Gemini, LiteLLM, OpenAI-compatible profiles.** No new controls unless the implementer verifies one in current documentation or the CLI's help; do not guess.

## Requests

- Anthropic: `thinking: {type: "adaptive"}`, `{type: "disabled"}`, or `{type: "enabled", budget_tokens}` from `thinking`; omitted when null. A `provider_options.thinking` value still wins, as today.
- OpenAI Responses: `reasoning.mode` and `reasoning.summary` alongside `reasoning.effort`.
- OpenRouter: its `reasoning` request object (`enabled: false` for `off`), per OpenRouter's documentation.
- Codex: the summary setting, if verified above.
- Resolve the new fields through the same scope chain as effort (conversation or mission, project, workspace; node, graph default, stylesheet for flows) and carry them into workflow runs the same way effort is carried today.

## Flows

- Accept any effort string in model stylesheet rules: remove `ALLOWED_REASONING_EFFORTS` (`crates/attractor-dsl/src/validation.rs` and `transforms.rs`). The model and provider decide what is valid.
- Add `thinking`, `thinking_budget_tokens`, `reasoning_mode`, and `reasoning_summary` as node attributes, graph defaults, and stylesheet properties wherever `reasoning_effort` is accepted today (`attractor-dsl`, `attractor-core/src/flow_definition.rs`, `attractor-runtime/src/flow_runtime.rs`, `records.rs`), validated with the same rules as stored settings.

## Picker

In the model picker, under the effort row, show only the controls the selected model offers:

- Thinking: a choice of Default, Adaptive, Off, and Budget, showing only the options offered. Budget reveals a token input with the 1024 minimum.
- Mode: Standard or Pro.
- Summary: Default, Auto, Concise, Detailed.

The inherited-choice rule from the effort row applies: these controls are locked until a model is chosen explicitly. Flow editor call sites map the new fields to and from the flow attributes, as they do for effort.

## Verification

- Settings: the new fields round-trip at every scope; invalid values are rejected; older documents load.
- Capabilities: each control appears only where its source says it is offered, including Anthropic from capabilities and notes, OpenAI modes and summaries from notes, and OpenRouter `mandatory`.
- Requests: each provider's field is sent when set and absent when null; `provider_options.thinking` still overrides for Anthropic.
- Flows: stylesheet rules accept any effort string; the new attributes validate, resolve through node, graph, and stylesheet, and reach the provider request.
- Picker: controls appear only for models that offer them, lock while inheriting, and save the expected values.
- `just test`.

## Out of scope

- Launch-time overrides for the new fields on flow run requests.
- Gemini thinking budgets, and any control not verified against current provider documentation.

## Hand-off

Write `changes/CR-2026-0129-provider-native-reasoning-controls/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
