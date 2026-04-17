# Chat Mode Collaboration Propagation Plan

## Summary

Restore the missing runtime invariant for Home Project Chat:

- the durable conversation `chat_mode` must control the app-server `turn/start.collaborationMode` for every assistant turn
- persisted `chat_mode` must not be treated as a UI-only or snapshot-only flag
- transport-level tests must fail if that propagation breaks again

This plan is intentionally narrow. It fixes the transport mismatch only. It does not redesign plan-mode UX, `request_user_input`, or the inline pending-question prototype.

## Problem

Spark currently persists `chat_mode` on the conversation state and exposes it in snapshots, but that value is dropped before the app-server call is made.

Observed repository facts:

- `ProjectChatService._prepare_turn()` computes `effective_chat_mode` and writes it into conversation state.
- `PreparedChatTurn` does not carry `chat_mode`.
- `ProjectChatService._execute_turn()` calls the session with only `prompt` and `model`.
- `CodexAppServerChatSession.turn()` does not accept a chat-mode argument.
- `CodexAppServerClient.run_turn()` builds `turn/start` without `collaborationMode`.

As a result, a thread can be durably marked `plan` while the actual Codex turn runs without Plan collaboration semantics.

## Goals

- Make `chat_mode` authoritative for app-server collaboration mode on every turn.
- Preserve the existing durable conversation behavior for `chat_mode` and `mode_change` turns.
- Keep the implementation small and local to the project-chat transport path.
- Add regression tests at the transport seam, not only at the persistence layer.

## Non-Goals

- No new chat UI or timeline behavior.
- No `request_user_input` implementation work.
- No changes to flow launches, run requests, or run-question answering.
- No attempt to infer collaboration mode from hidden app-server thread state.

## Implementation Plan

### 1. Carry effective chat mode through the prepared turn

Update the prepared-turn model so the execution layer receives the already-resolved mode.

Target change:

- add `chat_mode: str` to `PreparedChatTurn`
- populate it from `effective_chat_mode` in `ProjectChatService._prepare_turn()`

Why:

- `_prepare_turn()` is already the authoritative point where persisted thread mode and optional request override are reconciled
- downstream code should consume that resolved value rather than re-deriving it later

### 2. Thread chat mode through the session boundary

Update the project-chat execution path so the resolved mode reaches the app-server client.

Target change:

- change `ProjectChatService._execute_turn()` to call the session with `chat_mode=prepared.chat_mode`
- extend `CodexAppServerChatSession.turn()` to accept `chat_mode`
- forward that value to `CodexAppServerClient.run_turn()`

Why:

- this is the smallest point-to-point repair
- it keeps `ProjectChatService` as the owner of conversation semantics and the session/client layers as pure transport adapters

### 3. Send `collaborationMode` on every app-server turn

Extend the app-server client request builder so `turn/start` includes explicit collaboration mode derived from the Spark chat mode.

Required behavior:

- `chat` maps to Codex default collaboration mode
- `plan` maps to Codex plan collaboration mode
- the mode is sent on every `turn/start`, not only on thread creation or resume
- existing `model`, `cwd`, `approvalPolicy`, and sandbox behavior remain unchanged

Implementation notes:

- use the canonical app-server payload shape already exercised elsewhere in the repository for Codex collaboration modes
- do not rely on implicit app-server thread memory for mode continuity
- if a helper is needed for mapping Spark values to app-server payloads, keep it local to the app-server client layer

### 4. Add regression tests where the bug actually lives

Current project-chat tests prove persistence and ordering, but they do not prove transport propagation. Add focused tests that fail if the client omits `collaborationMode`.

Required coverage:

- client unit test: `CodexAppServerClient.run_turn()` includes `collaborationMode` in `turn/start`
- client unit test: both `chat` and `plan` mappings are covered
- session unit test: `CodexAppServerChatSession.turn()` forwards `chat_mode` to `run_turn()`
- project-chat test: a persisted `plan` thread with no per-request override still launches the turn with plan collaboration mode
- project-chat test: explicit `chat_mode="plan"` switch-and-send launches the same turn in plan mode
- project-chat test: `chat` mode launches with default collaboration mode rather than omitting the field

Test style constraints:

- assert on the actual `turn/start` payload or the call arguments that produce it
- do not settle for state-only assertions like `snapshot["chat_mode"] == "plan"`

### 5. Keep the fix narrow and compatible with current in-flight changes

There are already unrelated edits in the worktree, including plan-mode UI work and the inline pending-question preview. The implementation should:

- preserve unrelated modifications
- avoid reverting or reshaping concurrent plan-mode UI changes
- keep this change scoped to the chat transport path and its tests

## Target Paths

- `src/spark/workspace/conversations/models.py`
- `src/spark/chat/service.py`
- `src/spark/chat/session.py`
- `src/spark_common/codex_app_client.py`
- `tests/api/test_project_chat.py`
- transport-focused backend tests under `tests/api/` or the closest existing client/session test module

## Acceptance Criteria

- A conversation persisted as `chat_mode: "plan"` launches subsequent assistant turns with explicit plan collaboration mode at app-server `turn/start`.
- A conversation persisted as `chat_mode: "chat"` launches turns with explicit default collaboration mode at app-server `turn/start`.
- Explicit per-request `chat_mode` overrides continue to work and remain atomic with the user turn.
- Existing `mode_change` durability and snapshot behavior remain unchanged.
- New regression tests fail if `collaborationMode` is dropped anywhere between `_prepare_turn()` and `run_turn()`.

## Validation

- `uv run pytest -q`

## Out of Scope Follow-Up

If this transport fix lands cleanly, the next planning-only task can revisit the stricter product rule for plan mode itself:

- whether the Spark workspace assistant should refuse repository edits while the active conversation is in plan mode
- whether that behavior should be enforced by runtime collaboration mode, by explicit Spark prompt rules, or by both
