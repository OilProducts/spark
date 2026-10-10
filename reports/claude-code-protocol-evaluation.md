**Claude Code control protocol: what it can enable in Spark**

**Verdict: Claude Code's headless protocol can support nearly every agent feature Spark has specified or half-built. It also offers four Spark-native wins no other backend has. CR-0120 is the right first step, but it is scoped around the wrong decision.** It treats one process per turn as a latency trade-off, when process lifetime actually decides which of these features Spark can reach. Re-scope CR-0120 around a session host whose lifetime matches Spark's unit of work, and sequence the rest behind it.

**Sources:**
- Spark at `71685d83`.
- Claude Code's wire protocol as specified by the Agent SDK 0.3.281 type definitions (CLI 2.1.281, released 2026-09-23).
- The schema bundled in the installed CLI 2.1.220.
- The CHANGELOG between those two versions.

**Live probe** against the installed CLI:
- Confirmed: the stream-json handshake, several turns in one process, and the capabilities `interrupt_receipt_v1`, `interrupt_cancel_queued_v1` and `msg_lifecycle_v1`.
- Not tested: questions and interrupts. The CLI's OAuth session had expired, so every turn failed.

**Stability labels:**
- **Public:** in the SDK's published control/message unions or in `--help`.
- **Gated:** Public, and also advertised by a `capabilities` string at startup.
- **Hidden:** only in the CLI bundle or the minified SDK. Treat as unstable: `get_plan` and `get_workspace_diff` were dropped from the public union between 2.1.220 and 2.1.281.

---

**Findings that change the plan**

**F1: Process lifetime decides which features Spark can have.**

Claude Code runs many turns in one process while stdin stays open. Closing stdin makes the run one-shot, and one-shot runs don't let background work outlive them:
- Held-back background agents are killed when the result is released (sdk.d.ts ~L1734).
- Background shells are stopped shortly after the result (CHANGELOG).
- Background subagents are Claude Code's default.

The three options:

| Process model | What works | What breaks |
|---|---|---|
| One per turn, stdin closed (today) | Streaming output | Questions, interrupts, steering. Background subagents die with the turn. |
| One per turn, stdin open until the turn's work is done (CR-0120, amended) | Questions, interrupt, steering, queued messages | Nothing, *if* the turn ends when a `result` has arrived **and** the `background_tasks_changed` set is empty (ignoring `ambient` entries). Ending on the first `result` loses background work. |
| One per session (conversation lifetime) | Everything above, plus background work that outlives a turn and wakes the agent later (a task notification starts a new turn on its own) | Spark has no agent-initiated turns: every turn starts at `start_turn` with a pending assistant turn. This needs a conversation-model change first. |

Recommendation: **make the lifetime Spark's unit of work.**
- **Flow nodes:** one session per node execution. Clarifications, steering and contract-repair re-prompts become messages in that session. Today a repair re-prompt starts a new Claude session with no task context, because the codergen request carries no session id ([llm_backend.rs:490](../crates/spark-agent-adapter/src/llm_backend.rs:490)).
- **Chat:** one session per turn, ending by the rule above. Wait for agent-initiated turns before trying conversation-lifetime sessions.
- **Recovery:** after a crash or server restart, respawn with `--resume`. Spark already stores the session id.
- **Container nodes:** in container mode, Claude runs inside the container under `spark-server worker run-node`, so the session host lives in the worker. Answers, steering and interrupts must reach the worker, either over the worker protocol or through the run journal (which is how clarification answers already travel).

**F2: Spark can serve its own tools over the control channel.**

The mechanism is `initialize.sdkMcpServers` plus `mcp_message`: Spark answers MCP JSON-RPC itself, in-process (Public).

Today Spark's control surface is only a promise in the workspace frame: `spark` is on `PATH` and `SPARK_HOME` / `SPARK_API_BASE_URL` are set ([conversations.rs:3459](../crates/spark-workspace/src/conversations.rs:3459)). For Claude Code the promise is false (CR-0087), and in source checkouts the CLI refuses fallbacks.

Serving the same operations as MCP tools:
- Operations to cover: flow list/describe/validate, `convo run-request`, run retry/continue/events, and the task board.
- No dependency on `PATH`, environment variables or a built binary.
- The conversation handle is bound implicitly, and results are typed.
- Missing operations can be added without CLI work: run status, cancel, answering a run question.

Plan:
- Ship CR-0087 now as the short-term fix, and plan this as its replacement.
- The same tool handlers can later be served to Codex as a stdio MCP server.
- Cost: a small MCP handler (`initialize`, `tools/list`, `tools/call`). An unanswered handshake times out after 70s.

**F3: Node outcomes can be schema-enforced instead of parsed.**

`--json-schema` returns `result.structured_output`, validated inside the agent loop. If the model can't comply, the result subtype is `error_max_structured_output_retries` (Public).

Today `status_envelope` is prompt-and-parse with repair re-prompts ([status_envelope.rs:49](../crates/spark-agent-adapter/src/status_envelope.rs:49), [codergen.rs:884](../crates/spark-agent-adapter/src/codergen.rs:884)).

Generate the schema from the node:
- `outcome` as an enum.
- `preferred_label` as an enum of the outgoing edge labels.
- `suggested_next_ids` as an enum of the successor node ids.
- `context_updates` restricted to the node's `writes_context` keys, with `additionalProperties: false`.

Misspelled routing labels become impossible. Keep the parser for the other backends.

**F4: Hooks give unattended runs a policy without prompts.**

- **Today:** every trigger-launched run executes with `bypassPermissions` (Claude) or never-approve with full access (Codex), and there is no policy ([agent_settings.rs:133](../crates/spark-common/src/agent_settings.rs:133)).
- **Mechanism:** `PreToolUse` callbacks registered at `initialize` (Public) can allow a call, deny it with a reason, rewrite its input, or `defer` it to the host.
- **What Spark gains:** per-execution-profile rules (no writes outside the project, no `git push`, command allow-lists) without anyone approving prompts. `result.permission_denials` is the authoritative record of what was blocked.
- **To verify:** that a hook deny is honored under `bypassPermissions` (open question 2).
- **Separately:** the workspace frame belongs in `appendSystemPrompt`, not prepended to the first user message ([conversations.rs:1049](../crates/spark-workspace/src/conversations.rs:1049)). The CLI records the system prompt on the first request and reuses it on resume, which matches Spark's "frame only on a new thread" behavior.

**F5: Plan mode maps directly onto Spark's plan-to-CR workflow.**

- **How Spark works:** a plan segment becomes a proposed-plan artifact. Approving it writes `changes/CR-*/request.md` and launches implement-change ([conversations.rs:2021](../crates/spark-workspace/src/conversations.rs:2021), [conversations.rs:4423](../crates/spark-workspace/src/conversations.rs:4423)).
- **The gap:** Claude Code never emits the Plan channel today (CR-0082).
- **Starting plan mode:** run plan-mode turns with `--permission-mode plan`.
- **Receiving the plan:** it arrives as an `ExitPlanMode` permission request carrying the plan text. Emit it on the Plan channel.
- **Reject:** answer deny with the reviewer's note. The model revises the plan and stays in plan mode.
- **Approve:** answer deny with a note that the plan was handed to a flow, and end the turn. Implementation belongs to implement-change, not the chat.
- **Plan format:** `planModeInstructions` can make Claude write plans in the CR `request.md` format directly.

**F6: Defects and gaps found along the way** (independent of the protocol work):
- **Token usage is undercounted.** The adapter reads `result.usage` ([claude_code.rs:746](../crates/spark-agent-adapter/src/claude_code.rs:746)). The SDK documents that field as "MAIN AGENT LOOP ONLY — excludes Task subagent, sidechain, and auxiliary model calls" and says to prefer `modelUsage`.
  - Runs that use subagents or compaction under-report tokens and estimated cost.
  - Also, on 2.1.277+ `total_cost_usd` is cumulative across a resumed session, so never sum it across turns.
- **Node timeouts are ignored.** Codergen passes `timeout_seconds` ([llm_backend.rs:511](../crates/spark-agent-adapter/src/llm_backend.rs:511)), but the Claude adapter has no timeout. A stuck Claude node blocks its run indefinitely, and run cancel waits for the node boundary ([controls.rs:718](../crates/attractor-runtime/src/controls.rs:718)). Interrupt fixes both.
- **CR-0120's degraded mode can hang.** "Stop responding to control traffic" leaves any outstanding `can_use_tool` blocked forever, because permission prompts have no deadline (sdk.d.ts L207). Degrade by answering (allow for permission requests, an error for unknown subtypes), never by going silent.
- **An expired login fails opaquely.** The turn fails with `Failed to authenticate: OAuth session expired and could not be refreshed` (observed). Codex maps its equivalent to a reconnect prompt ([codex_app_server/auth.rs:13](../crates/spark-agent-adapter/src/codex_app_server/auth.rs:13)). The `initialize` response reports the account, so Spark can check before a turn and show a connection status like Codex's.
- **Personal config leaks into runs.** Claude is spawned with the user's full settings cascade: personal CLAUDE.md, hooks, plugins and MCP servers. Flow runs therefore aren't reproducible across machines. `--setting-sources`, `--strict-mcp-config` and the existing `CLAUDE_CONFIG_DIR` setting control this; choose a policy per execution profile.

---

**Capability map**

Value and cost are relative estimates for Spark.

*Conversations*

| Spark feature | State today | Protocol mechanism | Stability | Value / cost |
|---|---|---|---|---|
| Answer agent questions | Card and route exist; Claude returns an error ([llm_backend.rs:1085](../crates/spark-agent-adapter/src/llm_backend.rs:1085)) | `can_use_tool` for `AskUserQuestion`, answered with allow + `updatedInput.answers` | Public | High / Low |
| Stop a turn | No route, no trait method, no button | `interrupt` (optionally `cancel_queued`) | Gated | High / Low |
| Send while the agent works | Rejected with 409 ([conversations.rs:959](../crates/spark-workspace/src/conversations.rs:959)) | stdin user message with a `uuid`, folded in between tool rounds; `cancel_async_message` to withdraw; `result.user_message_uuids` to confirm delivery | Public (lifecycle frames Hidden) | High / Medium |
| Plan mode | `chat_mode` ignored for Claude | `--permission-mode plan`, `ExitPlanMode`, `planModeInstructions` | Public | High / Medium |
| Reasoning effort | Ignored for Claude; UI lists low…ultra ([models.rs:11](../crates/spark-workspace/src/models.rs:11)) | `--effort`; `list_models` returns each model's `supportedEffortLevels` | Public | Medium / Low |
| Context meter | Usage stored, never rendered | `get_context_usage`; per-message usage | Public | Medium / Low |
| Workspace frame | Prepended to the first message | `appendSystemPrompt` | Public | Medium / Low |
| Images and attachments | None | image and document content blocks | Public | Medium / Medium |
| Fork, edit and retry | None | `--fork-session`, `--resume-session-at`; `rewind_files` (needs an env var) | Public (`rewind_conversation` Hidden) | Low–Medium / Medium: Spark's transcript must fork too |

*Flow runtime*

| Spark feature | State today | Protocol mechanism | Stability | Value / cost |
|---|---|---|---|---|
| Clarification questions in nodes | Codex only (CR-0108) | `AskUserQuestion` routed to the existing clarification handler | Public | High / Low after F1 |
| Human steering | Stub that always rejects ([attractor-api lib.rs:2435](../crates/attractor-api/src/lib.rs:2435), [TODO.md:5](../TODO.md:5)) | User message into the live node session | Public | High / Low–Medium |
| Manager-loop steering | Broker has no Claude registration ([llm_backend.rs:245](../crates/spark-agent-adapter/src/llm_backend.rs:245)) | Same | Public | Medium / Low |
| Mid-node cancel and timeouts | Cancel only at node boundaries; timeouts ignored | `interrupt` | Gated | High / Low |
| Structured outcomes | Prompt, parse, repair | `--json-schema` | Public | High / Low |
| Session reuse across nodes | `thread_id` and fidelity computed but never used ([context.rs:361](../crates/attractor-runtime/src/context.rs:361)) | `--resume`, `--fork-session` | Public | Medium / Medium |
| Per-node guards | None | `--max-turns`, `--max-budget-usd`, `--tools` / `--disallowedTools` | Public | Medium / Low |

*Runs, usage, policy*

| Spark feature | State today | Protocol mechanism | Stability | Value / cost |
|---|---|---|---|---|
| Accurate tokens and cost | Undercounted (F6) | `modelUsage`, `total_cost_usd` | Public | High / Low |
| Rate limits | Not tracked | `rate_limit_event` (subscription accounts); lets triggers back off | Public | Medium / Low |
| Retry and compaction status | Passed through as `Other(...)`, never rendered | `api_retry`, `status`, `compact_boundary`, mapped onto the existing ContextCompaction kinds | Public | Medium / Low |
| Subagent visibility | Flat tool rows | `task_*` frames, `parent_tool_use_id` | Public | Medium / Medium (nesting in segments and UI) |
| Policy for unattended runs | Bypass everywhere | `PreToolUse` hook callbacks; `permission_denials` | Public | High / Medium |
| Isolation for native runs | None ([TODO.md:47](../TODO.md:47)) | sandbox via `--settings`; `--restricted` | Public | Medium / Low (Claude-only) |
| Spark tools for the agent | CLI on `PATH`; broken for Claude (CR-0087) | `sdkMcpServers` + `mcp_message` | Public | High / Medium |
| Connection status | None | `initialize` response `account`; assistant error codes | Public | Medium / Low |

---

**Do not adopt**
- **Per-tool approval UI.** The spec makes approvals artifact-level ([spark-workspace.md §12](../specs/spark-workspace.md:268)), and hook policy covers unattended safety. Revisit if users ask for it.
- **`request_user_dialog`.** Its dialog kinds are undocumented, and the type definitions contradict each other on how to answer an unknown kind. Declaring no kinds makes it fail closed.
- **Hidden requests:** `command_lifecycle`, `rewind_conversation`, `end_session`, `side_question`, `claude_authenticate`. For delivery tracking, use `result.user_message_uuids` and `queued_turn_count`, which are Public.
- **Minor extras:** MCP app resources, prompt suggestions, memory recall, output styles, plugins via `--await-initialize`, and keeping warm processes around purely for latency.

**Backend neutrality**
- **Belongs on `AgentTurnBackend` as backend-neutral methods:** questions, interrupt, steering and queued messages. The Codex app-server has `turn/interrupt` and `turn/steer`, so Spark's Codex path gains Stop and mid-node cancel from the same work.
- **Shared by both backends:** the MCP tool surface from F2.
- **Claude-specific adapters behind existing Spark concepts:** structured output (node outcome), hooks (policy) and `ExitPlanMode` (the Plan channel).

---

**Recommended sequence**

**0. Housekeeping** (independent and small):
- Re-login the Claude CLI.
- Raise the supported floor to **2.1.281**. Relevant releases:
  - 2.1.281: fixes stream-json sessions failing every turn after a plain-string assistant message.
  - 2.1.277: `-p` no longer hangs after an internal error.
  - 2.1.268: PermissionRequest hooks now fire in `-p`.
  - 2.1.259: adds `--permission-prompts`.
- Switch usage accounting to `modelUsage`.
- Ship CR-0087.

**1. Session host** (re-scoped CR-0120):
- A stdin-open session per unit of work, with uuid-stamped messages.
- A control router that always answers.
- Capability gates, the turn-end rule from F1, and respawn via `--resume`.
- Features that are thin on top of the host:
  - `AskUserQuestion` routed to both chat request-user-input and the flow clarification handler.
  - Interrupt, wired to chat Stop, mid-node run cancel and node timeouts.

**2. Conversation controls:**
- Queued messages instead of the 409.
- Human and manager-loop steering.
- Plan mode via `ExitPlanMode`, with `planModeInstructions`.
- The effort flag.

**3. Structured outcomes:** `--json-schema` for `status_envelope` nodes. Repairs become in-session messages.

**4. Spark tools and policy:**
- MCP tools over the control channel, replacing CR-0087's mechanism.
- Hook-based policy per execution profile.
- Explicit settings sources, so runs don't pick up personal config.

**5. Observability:**
- Map retries, status, compaction and rate limits.
- Context meter.
- Subagent nesting.

**6. Later:**
- Session reuse across nodes via fidelity and `thread_id`.
- Conversation fork.
- Attachments.
- Agent-initiated turns, and then session-lifetime chat.

---

**Open questions for the live probe** (after re-login, against 2.1.281)
1. Do `AskUserQuestion` and `ExitPlanMode` reach `can_use_tool` under `bypassPermissions`? If not, run with `default` and an allow-all responder.
2. Is a `PreToolUse` deny honored under `bypassPermissions`?
3. With stdin open, does a background subagent's completion start its own turn and `result`? Does `background_tasks_changed` go empty before the final one?
4. Does `priority: "now"` interrupt a headless turn, or must Spark send `interrupt` first?
5. Is `msg_lifecycle_v1` still advertised on 2.1.281?
