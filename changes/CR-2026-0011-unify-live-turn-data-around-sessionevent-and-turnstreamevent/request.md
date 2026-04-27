# Unify Live Turn Data Around `SessionEvent` and `TurnStreamEvent`

## Summary
Move the architecture toward one clean boundary: `agent.SessionEvent` carries the generic agent/model stream information, and Spark converts that into `TurnStreamEvent` at the Spark boundary. `TurnStreamEvent` remains the backend/frontend live-turn contract and the materializer input for Home chat, with Codex JSON-RPC normalized directly into that same shape because Codex is an alternate runtime path, not part of `agent`.

This change should remove the current compatibility shims and avoid adding another adapter layer.

## Key Changes
- Enrich `agent.SessionEvent` so it preserves the important unified-LLM stream concepts:
  - Add `ASSISTANT_REASONING_START`, `ASSISTANT_REASONING_DELTA`, `ASSISTANT_REASONING_END`.
  - Add `MODEL_TOOL_CALL_START`, `MODEL_TOOL_CALL_DELTA`, `MODEL_TOOL_CALL_END` for model-proposed tool calls, distinct from actual tool execution events.
  - Add `MODEL_USAGE_UPDATE` for token/usage data from provider finish events.
  - Ensure existing assistant text end events carry `response_id` consistently.

- Update `agent.Session._stream_response()` to forward more of `unified_llm.StreamEvent`:
  - Text events continue to emit existing assistant text events.
  - Reasoning events emit the new assistant reasoning events.
  - Provider/model tool-call stream events emit `MODEL_TOOL_CALL_*`.
  - Finish usage emits `MODEL_USAGE_UPDATE`.
  - Existing actual tool execution events remain unchanged.

- Keep `unified_llm.StreamEvent` internal to the agent/unified-LLM layer:
  - Do not create a direct `unified_llm.StreamEvent -> TurnStreamEvent` path in Spark.
  - Spark should consume `SessionEvent`, not unified-LLM provider events.

- Tighten `TurnStreamEvent` as the Spark boundary object:
  - Remove constructor/property compatibility shims such as direct `app_turn_id`, `item_id`, `response_id`, `raw_kind`, `text`, and `item`.
  - Require correlation data to live under `event.source`.
  - Update Spark chat service/session code to read `event.source.*`.
  - Keep Codex normalization direct: Codex JSON-RPC events become `TurnStreamEvent` with `source.backend = "codex_app_server"`.

- Update Spark’s `SessionEvent -> TurnStreamEvent` conversion:
  - Assistant text maps to `content_delta` / `content_completed`, channel `assistant`.
  - Assistant reasoning maps to `content_delta` / `content_completed`, channel `reasoning`.
  - Actual tool execution events map to tool-call lifecycle events.
  - `MODEL_USAGE_UPDATE` maps to `token_usage_updated`.
  - `MODEL_TOOL_CALL_*` is carried through tests/specs as available session data, but is not rendered as a Home chat tool row in this change.

## Specs
- Update the agent/coding-agent specs to document the expanded `SessionEvent` kinds and the distinction between model-proposed tool calls and actual tool execution.
- Update `specs/spark-workspace.md` to state:
  - `SessionEvent` is the generic agent runtime stream.
  - `TurnStreamEvent` is the Spark live-turn/materialization boundary.
  - Codex normalizes directly to `TurnStreamEvent`.
  - Spark does not adapt `unified_llm.StreamEvent` directly for chat rendering.
- Leave the Runs Progress UI implementation out of this change, but preserve the forward path: future Runs Progress should render persisted/live `TurnStreamEvent`-derived content with the same markdown semantics as Home chat.

## Test Plan
- Add or update agent tests proving `Session` emits:
  - assistant reasoning events from unified-LLM reasoning stream events,
  - model tool-call stream events separately from execution tool events,
  - usage update events from finish/usage data,
  - `response_id` on assistant text completion.

- Update Spark chat tests proving:
  - `SessionEvent` assistant text still materializes into assistant content,
  - reasoning `SessionEvent`s materialize into reasoning content,
  - usage events update token accounting,
  - execution tool events still render as tool activity,
  - model-proposed tool-call events do not create misleading execution rows.

- Update Codex tests to use the strict `TurnStreamEvent.source` shape and verify Codex still emits the same live content/tool/usage semantics.

- Run:
  - `uv run pytest -q tests/spark_common/test_codex_app_client.py tests/api/test_project_chat.py -x --maxfail=1`
  - `uv run pytest -q tests/api/test_backend_invariance.py -x --maxfail=1`
  - targeted agent/session streaming tests
  - final `uv run pytest -q`

## Assumptions
- `SessionEvent` should stay generic to the agent layer and should not gain Spark-specific fields.
- `TurnStreamEvent` is allowed to be Spark-specific because it is the Spark backend/frontend and materialization boundary.
- Codex remains a supported peer runtime path and does not need to be forced through `agent.Session`.
- This change should remove compatibility shims rather than introduce another normalization object.
