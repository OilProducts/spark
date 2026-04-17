# Set Global Settings Default Model to `gpt-5.4`

## Summary

Change the app-wide UI defaults so the Global Settings tab seeds `gpt-5.4` as the default LLM model for clean state. Keep existing persisted user preferences unchanged. Also update the Settings tab’s model placeholder and model suggestions so the UI reflects the new default consistently.

Assumption: your “got 5.4” request means `gpt-5.4`.

## Key Changes

- Update `DEFAULT_UI_DEFAULTS` in `frontend/src/state/store-helpers.ts` so `llm_model` defaults to `gpt-5.4` for users with no saved `spark.ui_defaults` local storage entry.
- Keep `llm_provider` and `reasoning_effort` behavior unchanged unless current code already relies on paired defaults.
- Update the Global Settings model input in `frontend/src/features/settings/SettingsPanel.tsx` so the placeholder matches the new default (`gpt-5.4` instead of `gpt-5.2`).
- Update `frontend/src/lib/llmSuggestions.ts` so `gpt-5.4` is included in the OpenAI suggestion list; place it in a sensible default-facing position near the top.
- Do not add a migration that overwrites existing saved UI defaults in local storage.

## Public Interface / Behavior Changes

- The Global Settings tab will show `gpt-5.4` as the default model for fresh browser state.
- New flows that snapshot global defaults, when created from clean state without prior saved settings, will inherit `gpt-5.4`.
- Existing users with persisted `spark.ui_defaults` keep their current chosen model unchanged.

## Test Plan

- Update frontend tests that currently assume an empty global default model or a `gpt-5.2` placeholder.
- Add or adjust a store-helper test to verify `loadUiDefaults()` returns `gpt-5.4` when local storage has no saved UI defaults.
- Update the Settings panel test to assert the model input reflects the new placeholder/default behavior.
- Verify any editor/graph settings tests that depend on inherited global defaults still pass with the new seeded value.
- Run `uv run pytest -q` before reporting completion, per repo policy.

## Assumptions

- “Default model on the global settings tab” refers to the app-wide `uiDefaults.llm_model` used by the Settings tab and flow snapshotting, not per-flow `ui_default_llm_model` values already stored in DOT files.
- Existing persisted local-storage preferences should not be overridden.
- No backend/API contract change is required; this is a frontend defaulting change only.
