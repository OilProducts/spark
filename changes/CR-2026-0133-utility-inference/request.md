# Utility inference

## Summary

Add utility inference: a single model call that Spark's own code can make for small housekeeping jobs, such as naming a thread, using a utility model the user configures in Settings. Each use supplies its own prompt in code. The first use gives a conversation a generated title after its first completed turn.

## Why

Spark has no way to ask a model for a small piece of text outside a chat turn or a flow run. Chat turns and flow nodes run full agent sessions on the chat model, which is too slow and costly for jobs like a thread title. Today a thread's title is its first user message cut to 64 characters (`derive_conversation_title` in `crates/spark-workspace/src/conversations.rs`).

## Setting

- Add a workspace setting for the utility model with the same fields as the chat model settings (`ModelSettings` in `crates/spark-common/src/settings.rs`: provider, profile, model, and reasoning controls). Store it in `spark.toml` as its own section, return it from the workspace settings API, and validate it as model settings are validated.
- Utility inference is off while no utility model is set. Spark does not fall back to the chat model.
- Settings shows a Utility model picker using the existing model chooser. It offers the models the workspace can reach, as the chat picker does (`chat_models` in `crates/spark-workspace/src/models.rs`).

## Invocation

- One function in `spark-workspace` makes a utility call. It takes instructions and input from the caller and returns the model's text. It returns a distinct "not configured" result when no utility model is set, and an error on failure.
- Prompts are constants in the calling code. They are not settings and are not user-editable.
- A call runs without tools, without history, and without persisting a session. It has a bounded output and a timeout. It creates no conversation, run, or transcript.
- Every provider the chat picker offers works:
  - API providers and LLM profiles: a single completion through `unified_llm_adapter::Client`, with credentials and profiles resolved the same way chat resolves them (`client_for_execution` in `conversations.rs`).
  - Claude Code and Codex: their existing paths start full agent sessions with tools in the project directory (`run_claude_code_codergen` and `run_codex_app_server_codergen` in `crates/spark-agent-adapter/src/llm_backend.rs`), so add a one-shot mode instead. It runs outside the project directory, with no tools, without loading project instructions, and without persisting a session. For example `claude -p --tools "" --no-session-persistence --system-prompt ...` and `codex exec --ephemeral --sandbox read-only --skip-git-repo-check -o ...`. Verify the flags against the installed CLIs.
- Reasoning controls in the setting are sent in each provider's own field, as for chat.

## First use: conversation titles

- After a conversation's first completed turn, if the conversation has no stored `title`, run a utility call in the background with the first user message and the first reply, and store the result as the conversation's `title`.
- The turn does not wait on it. Generate a title once per conversation. If utility inference is off or the call fails, keep the derived title and do not retry.
- The thread list shows the new title without a reload. Do not change the thread list's layout.

## Verification

- Setting: round-trips through the settings API, rejects invalid values, and older `spark.toml` files load unchanged. The picker offers the same models as the chat picker.
- Invocation: returns "not configured" when unset. The API-provider path sends one completion with no tools. The Claude Code and Codex one-shot commands are built with no tools, no session persistence, and a working directory outside the project. A timeout and a provider error surface as errors.
- Titles: a first completed turn stores a generated title once, without delaying the turn. A second turn does not regenerate it. A conversation that already has a stored title is untouched. With the feature off or failing, the derived title remains.
- `just test`.

## Out of scope

- Other uses of utility inference; each adds its own prompt when needed.
- Project- or conversation-level utility model settings.

## Hand-off

Write `changes/CR-2026-0133-utility-inference/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
