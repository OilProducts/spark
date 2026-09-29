---
id: CR-2026-0133-utility-inference
title: Utility inference, with generated thread titles as its first use
status: completed
type: feature
changelog: public
---

## Summary

Spark can now make a single small model call for its own housekeeping, using a **Utility model** chosen in Settings. The setting has the same fields as the chat model settings. It is stored in its own `[utility_models]` section of `spark.toml`. Utility inference is off until a utility model is set, and it never falls back to the chat model.

`spark_workspace::utility::utility_complete(settings, instructions, input)` makes the call. It returns `Ok(Some(text))`, returns `Ok(None)` when no utility model is set, and returns an error when the provider fails, the call times out (60s), or the model returns no text. API calls stream with an abort signal, so a timeout cancels the in-flight request. A call has no tools, no history, and no persisted session, and its output is bounded. It creates no conversation, run, or transcript.

- **API providers and LLM profiles** send one completion through `unified_llm_adapter::Client`. The client is built by chat's `client_for_execution` from the same `spark.execution.settings` snapshot a chat turn captures, so credentials, profiles, and reasoning controls resolve the same way.
- **Claude Code and Codex** use a new one-shot mode that runs in a fresh empty temporary directory outside the project:
  - Claude Code: `claude -p --setting-sources "" --tools "" --strict-mcp-config --no-session-persistence --output-format text --system-prompt …`. With no setting sources, user, project and local settings (hooks, permissions, plugins) stay out of the call; sign-in is unaffected.
  - Codex: `codex exec --ephemeral --sandbox read-only --skip-git-repo-check --ignore-rules -C <tmp> -o <file> -c developer_instructions=… -`

  Both flag sets were checked against the installed CLIs (Claude Code 2.1.282, codex-cli 0.154.0) with live calls.

**First use: conversation titles.** After a conversation's first completed turn, including a first turn that stopped on a question and was resumed by the answer, if the conversation has no stored title, a background thread sends the first user message and the first reply to the utility model and stores the result as the title. A stored title is any title other than the one derived from the first message. The turn does not wait for this call. The new title reaches every client's thread list through a new live `conversation.summary_upsert` event (`include_conversations=true`), and the open thread through `conversation.snapshot`. The list layout is unchanged. A title is generated only when exactly one turn has completed. If utility inference is off or the call fails, the derived title stays and later turns do not retry.

## Validation

- `spark-agent-adapter` `process_contracts/llm_backend_contracts.rs`:
  - The API path sends exactly one completion with no tools and no tool choice. It sends a system and a user message, reasoning controls in their own request fields, and `max_tokens` bounded to 4096 plus any thinking budget.
  - A provider error surfaces as an error, and a slow provider times out.
  - The Claude Code and Codex commands are built with the one-shot flags above, no `--resume`, and a working directory outside the project.
  - A fake CLI shows the call runs in a temporary directory that is removed afterwards. A non-zero exit reports stderr, a hung CLI is killed at the timeout, and empty output is an error.
- `spark-workspace` `process_contracts/utility_contracts.rs`:
  - Settings: the setting round-trips and clears. Seven kinds of invalid values are rejected, and a hand-edited invalid section is reported rather than used. An older `spark.toml` without the section loads byte-for-byte unchanged, and the chat model settings are unaffected.
  - Invocation: the call returns "not configured" when unset, returns the configured model's text, and surfaces a CLI failure as an error.
  - Titles: the first turn returns before a one-second utility call finishes, and the generated title then arrives as a `conversation_snapshot` and is stored. A second turn does not call the utility model again. A title set by hand is untouched. With the feature off, or with a failing call, the derived title remains, and a later turn does not retry.
- `spark-http` `contracts/workspace_route_contracts.rs`: `utility_models` round-trips through `GET`/`PATCH /workspace/api/settings`, rejects invalid values with 400, and clears with `null`.
- Frontend:
  - `SettingsPanel.test.tsx`: the Utility model card starts off. Its picker offers exactly the same options as the workspace chat picker, because both use the same `useModelOptions` discovery. The card saves a chosen model and clears it back to off.
  - `projectsHomeState.test.ts`: a later snapshot carrying a generated title updates the thread-list summary.
- `just test`: passed.

## Shipped Changes

- `spark-storage`: core settings validation covers the optional `utility_models` section.
- `spark-workspace/settings.rs`:
  - New `utility_models` settings section. `None` turns the feature off.
  - The section is validated like model settings, and saved under the profile-reference lock so LLM-profile deletion checks see it.
  - `workspace_settings` returns a `utility_models` view.
  - New `workspace_utility_model_settings` function.
- `spark-workspace/utility.rs` (new): `utility_complete`.
- `spark-workspace/conversations.rs`:
  - The execution-settings capture is extracted into `capture_execution_settings`, shared by chat turns and utility calls.
  - After a completed turn, a background `generate_conversation_title` runs with a code-constant prompt.
- `spark-agent-adapter/llm_backend.rs`: `UtilityCall`, `run_utility_call`, `utility_llm_request`, and `utility_cli_command` (the Claude Code and Codex one-shot mode).
- `spark-http`: the settings PATCH route publishes `utility_models` changes. The turn and answer routes give the conversation service a live publisher.
- `spark-workspace/live.rs`: `include_conversations` live query and `conversation_summary_envelope`.
- Frontend:
  - `UtilityModelSettingsEditor` (on/off switch plus the existing `ModelChooser`) on the Models & accounts tab. Turning it on selects GPT-5.6-Luna when Codex is available, otherwise Sonnet 5.5 when Claude Code is available, otherwise Codex's default.
  - The live controller subscribes to `conversation.summary_upsert` and updates loaded thread lists (`upsertHomeConversationSummary`), ignoring older revisions.
  - `fetchModelSettings`, `saveModelSettings`, and `useModelSettingsEditor` take a settings section.
