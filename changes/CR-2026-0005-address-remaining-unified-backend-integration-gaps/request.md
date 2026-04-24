# Address Remaining Unified Backend Integration Gaps

## Summary
Fix the remaining integration risks without disturbing the current Codex app-server path: durable context for unified project chat, reliable failure persistence, timeout enforcement, independent model discovery, and consistent `xhigh` reasoning support.

## Key Changes
- Add persisted-history hydration for `UnifiedAgentChatSession`.
  - Convert prior completed persisted user/assistant message turns into agent `UserTurn` / `AssistantTurn` history.
  - Build that history before appending the current user turn, so the current prompt is not duplicated.
  - Keep Codex session/thread behavior unchanged and do not reintroduce `{{recent_conversation}}`.

- Make chat failures persist for all normal exceptions.
  - Broaden assistant-turn failure handling from only `RuntimeError` to `Exception`.
  - Mark the pending assistant turn `failed`, store the error message, then re-raise for existing sync/background behavior.

- Enforce node timeouts in the unified pipeline backend.
  - Wrap the entire unified backend session run, including repair attempts, in `asyncio.wait_for`.
  - Return a runtime failure like `unified-agent backend timed out after Ns`.
  - Preserve session/client cleanup on timeout or cancellation.

- Decouple chat model discovery from the Codex app-server.
  - Build unified provider model metadata independently.
  - Attempt Codex app-server model listing in isolation; if unavailable, log and return unified models rather than failing the whole endpoint.
  - Keep the response shape unchanged.

- Align `xhigh` reasoning effort across Attractor and UI surfaces.
  - Add `xhigh` to stylesheet validation, DOT/default-model validation, frontend stylesheet preview, and graph settings default reasoning choices.
  - Keep existing defaults unchanged.

- Small cleanup: update the unsupported codergen backend error message to include the new `provider-router` backend.

## Test Plan
- Add project-chat tests for unified history hydration across recreated sessions, ensuring prior complete user/assistant turns are present and the current message is not duplicated.
- Add project-chat failure tests using a fake unified session that raises a non-`RuntimeError`, asserting the assistant turn is marked failed.
- Add unified pipeline backend timeout tests with a fake slow session, asserting runtime failure and cleanup.
- Add model-listing tests where Codex app-server discovery fails but unified models are still returned.
- Add validation/UI tests for `reasoning_effort: xhigh` in stylesheet/default-model paths.
- Before completion, run `uv run pytest -q` per repo policy; also run the relevant frontend unit test command for touched frontend code.

## Assumptions
- Codex app-server remains the default/current functional path.
- Unified chat history v1 replays text user/assistant turns only; durable replay of tool calls, gates, and raw provider internals stays out of scope.
- No public API schema changes are needed for these fixes.
