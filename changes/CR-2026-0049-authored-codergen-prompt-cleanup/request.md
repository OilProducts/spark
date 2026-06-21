# Authored Codergen Prompt Cleanup

## Summary
Port Metalshop’s authored-attribute prompt behavior into Spark so codergen prompts only come from explicitly authored `prompt` or `label` values. Default/generated labels and node IDs must no longer become implicit codergen prompts.

## Key Changes
- Add `has_authored_non_empty_attr(node, key)` to the DOT model layer.
- Update codergen prompt resolution in both places that currently choose fallback prompt text:
  - executor `_prompt_for_node(...)`
  - `CodergenHandler.execute(...)`
- New behavior:
  - If `prompt` is authored and non-empty, use it.
  - Else if `label` is authored and non-empty, use it.
  - Else use an empty prompt.
  - Do not fall back to defaulted label values.
  - Do not fall back to `node_id`.
- Keep non-codergen prompt behavior unchanged.

## Related Codergen Polish
- Pass `emit_event` through the filtered backend kwargs instead of as an unconditional call argument, so backend compatibility is decided by `_filter_backend_kwargs`.
- Use backend `llm_profile` as the fallback profile when resolving effective `llm_profile`.
- Update codergen read-contract parse error wording from `spark.reads_context parse error` to `context.reads contract parse error`.

## Tests
- Add/adjust tests proving:
  - authored `prompt` still wins.
  - authored `label` is used only when `prompt` is blank.
  - default/generated label does not become the prompt.
  - codergen with no authored prompt/label passes an empty prompt to the backend.
  - `node_id` is not used as a prompt fallback.
  - backends without `emit_event` still work through filtered kwargs.
  - backend `llm_profile` fallback is honored.
  - read-contract parse error text uses the new wording.
- Remove or rewrite any tests that assert node-id/default-label prompt fallback.
- Run `uv run pytest -q`.

## Assumptions
- This is a behavior cleanup, not a compatibility migration; old implicit node-id prompt behavior should be removed.
- The current in-progress CR-0048 steering work should be completed or committed first, then this change should be made as a separate commit.
