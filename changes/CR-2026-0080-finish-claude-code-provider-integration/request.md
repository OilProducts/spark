# Finish Claude Code Provider Integration

## Summary

CR daba7b97 added a `claude-code` codergen backend that drives the Claude Code CLI (`claude -p`, stream-json) for `agent_task` pipeline nodes. The provider is currently unreachable everywhere else: project chat rejects it at validation, the chat and graph-editor UIs do not offer it, and the session-resume hook reads a metadata key nothing writes. Worse, the profile normalizer silently maps `claude_code` to the plain `anthropic` HTTP profile, so a hand-configured chat conversation would quietly bill an API key — the exact credential path the backend exists to avoid. Finish the integration: wire the chat conversation path, make the provider selectable in the UI, and carry the CLI session id across conversation turns.

## Background (current state, main branch)

- `crates/spark-agent-adapter/src/claude_code.rs` — working CLI driver used only by `run_claude_code_codergen` (`llm_backend.rs:81`). Reads metadata key `claude_code.session_id` to pass `--resume`; no code writes that key.
- `crates/spark-agent-adapter/src/llm_backend.rs:1011` (`RustLlmAgentTurnBackend::run_turn_with_event_sink`) and `:1052` (`answer_request_user_input`) dispatch codex explicitly; every other provider falls through to `build_agent_session`, whose normalizer (`profiles/openai.rs:35`) maps `claude_code` → the `anthropic` HTTP profile.
- `crates/spark-workspace/src/conversations.rs` — `validate_provider` (~line 5310) rejects `claude-code`; the runtime-session sidecar (`resume_codex_app_thread_id`, `record_runtime_session_thread`) persists codex thread ids across turns and feeds them back via metadata key `spark.runtime.codex_app_server.thread_id`.
- `crates/spark-workspace/src/models.rs` — `chat_models` aggregates codex discovery, unified public models, and configured profiles; nothing yields `claude-code` entries.
- `frontend/src/lib/llmSuggestions.ts` — `LLM_PROVIDER_OPTIONS` lacks `claude-code`; this list feeds both the chat provider dropdown (`useProjectsHomeController.ts`) and the graph editor settings (`GraphSettingsSections.tsx`).
- Test seam: `crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs` plus `tests/process_contracts/claude_code_contracts.rs` cover the codergen path against a fake binary; `SPARK_CLAUDE_CODE_BIN` selects the binary, `SPARK_FAKE_CLAUDE_CODE_LOG` records argv, `SPARK_FAKE_CLAUDE_CODE_MODE` scripts behavior.

## Requirements

### 1. Chat conversation dispatch

- In `RustLlmAgentTurnBackend::run_turn_with_event_sink`, route requests whose provider selector is `claude-code`/`claude_code` (case-insensitive, matching the existing `is_claude_code_provider_selector` semantics) to `ClaudeCodeBackend::run_agent_turn_with_event_sink`, mirroring the codex dispatch immediately above it. Match on the provider selector only, not `llm_profile`.
- Map `ClaudeCodeError` to `AgentError` preserving `message`, `retryable`, and `details` (as `raw`). Do not flatten `retryable`.
- `answer_request_user_input` must return a clear non-retryable `AgentError` for claude-code (the CLI print mode never emits request-user-input events) instead of falling through to the HTTP session path.
- Bare `anthropic` and `claude` selectors must keep their current behavior (HTTP adapter via the anthropic profile). Only the `claude-code`/`claude_code` selector changes routing.
- `validate_provider` in `conversations.rs` accepts `claude-code` and its error message lists it. Do not add `claude-code` to the "requires an explicit model" set — an empty model is valid and means the CLI's default model.

### 2. Session continuity across conversation turns

- After a successful claude-code chat turn, record `AgentTurnOutput.app_thread_id` (the CLI session id) in the conversation's runtime-session sidecar exactly as codex thread ids are recorded today; reuse the existing sidecar read/write helpers rather than inventing a parallel store.
- On the next turn of the same conversation, deliver the recorded session id to the backend under metadata key `spark.runtime.claude_code.session_id`. `ClaudeCodeBackend` must read that key first, keeping the existing `claude_code.session_id` key as an explicit-override fallback.
- Resume failure recovery: when the backend launched with `--resume` and the process exits without producing a `result` event, retry the turn exactly once in a fresh session (no `--resume`) before reporting failure. When the fresh retry is taken, the turn output must carry the new session id so the sidecar stops feeding the dead one. Surface the discarded-session fact through `AgentTurnOutput.thread_resume_failure` so the journal records it, matching the codex resume-failure shape.

### 3. UI selectability

- Add `'claude-code'` to `LlmProviderKey` and `LLM_PROVIDER_OPTIONS` in `frontend/src/lib/llmSuggestions.ts`, with `LLM_MODELS_BY_PROVIDER['claude-code'] = ['opus', 'sonnet', 'haiku']` (the CLI accepts these aliases; blank remains valid and uses the CLI default).
- Chat provider dropdown label: `Claude Code` (extend the label mapping in `useProjectsHomeController.ts`).
- The codex-specific model-discovery gating in `useProjectsHomeController.ts` (`isCodexProvider`, `isChatModelReady`, availability messages) must not gate claude-code; it behaves like the other non-codex providers (free-text model with suggestions).
- `chat_models` in `crates/spark-workspace/src/models.rs` returns static `claude-code` entries for the same three aliases (provider `claude-code`, no reasoning-effort support, none marked default) so the chat model chooser is populated; do not add a discovery/status probe for the CLI binary.
- The graph editor picks the new provider up through the shared `LLM_PROVIDER_OPTIONS`; verify an `agent_task` node can set `llm_provider: claude-code` in the editor without new validation diagnostics.

### 4. Fake-binary test seam

- Extend `spark-agent-fake-claude-code` with a mode where, if argv contains `--resume`, it exits non-zero with a stderr message and emits no `result` event, and otherwise completes successfully with a fresh session id. Use it to cover the resume-failure retry contract.

## Non-goals

- Steering/intervention-broker registration for claude-code codergen nodes (documented out of scope in the original commit).
- `reasoning_effort` mapping to the CLI.
- Turn timeouts or child-process cancellation (parity with codex today).
- Session resume for pipeline (`agent_task`) nodes across retries or runs.
- CLI model discovery, provider preflight checks, and documentation updates in the guides.

## Test Plan

- Adapter process contracts (fake binary): a chat-shaped `AgentTurnRequest` with provider `claude-code` routes through the CLI and returns mapped events, final text, usage, and `app_thread_id`; metadata `spark.runtime.claude_code.session_id` produces `--resume <id>` in the recorded argv; the resume-failure mode retries once without `--resume`, succeeds, and reports `thread_resume_failure` plus the new session id; `answer_request_user_input` returns the specified error.
- Workspace contracts: `validate_provider` accepts `claude-code` and still rejects unknown providers; a conversation's second turn feeds the recorded session id back through metadata; `chat_models` includes the three static claude-code entries.
- Regression: provider selectors `anthropic` and `claude` still resolve to the HTTP anthropic profile in both chat and codergen paths; codex chat dispatch is unchanged.
- Frontend unit tests: provider option and label present; model suggestions for `claude-code` are the three aliases; codex discovery gating does not apply to claude-code.
- Run the full repository validation gate: `just test` (Rust formatting check, workspace tests with all features, frontend unit tests, frontend build).

## Assumptions

- The Claude Code CLI is installed and logged in on hosts that select this provider; a missing binary keeps surfacing the existing configuration error from `claude_code.rs`.
- `--resume` with an unknown session id exits without emitting a `result` event; the retry-once-fresh rule keys off that observable behavior rather than parsing stderr text.
- Static model aliases (`opus`, `sonnet`, `haiku`) are acceptable suggestions; exact model ids typed free-form pass through to `--model` unchanged.
- The runtime-session sidecar is conversation-scoped and provider-agnostic enough to store one active session id per conversation; switching a conversation's provider discards resume continuity, which is acceptable.
- Known baseline flake: `workspace_trigger_route_contracts::webhook_dispatch_returns_while_the_run_still_executes` (spark-http) is timing-sensitive and can fail under parallel test load while passing in isolation; a failure only in that test on an otherwise-green gate is pre-existing, not caused by this change.
