# Coding Agent Rust Runtime Architecture

This architecture implements the Coding Agent Loop specification for the Rust rewrite worktree. The product behavior and external contract come from `specs/coding-agent-rust-runtime/source.md`, with extracted requirements in `specs/coding-agent-rust-runtime/requirements.json` and binding implementation decisions in `specs/coding-agent-rust-runtime/contract-decisions.json`.

The implementation target is a Rust-owned coding-agent runtime for normal Spark chat, agent-turn, and codergen-adjacent execution. Python `src/agent` modules may remain as oracle, compatibility, or historical implementation assets, but the normal distributed runtime must not import or dispatch through them. LLM calls go through `crates/unified-llm-adapter` low-level `Client.complete` or `Client.stream`; the coding-agent loop owns tool rounds, truncation, steering, lifecycle events, loop detection, subagents, and shutdown.

## Runtime Boundary

`crates/spark-agent-adapter` is the canonical Rust owner of the coding-agent library. It provides both the programmable session API described by the spec and Spark-facing adapter DTOs for project chat, agent turns, and codergen integration.

`crates/unified-llm-adapter` remains the LLM boundary. It owns provider clients, provider/model/profile resolution, request/response types, `Tool`, `ToolCall`, `ToolResult`, `StreamEvent`, `Usage`, and low-level complete/stream dispatch. It must not absorb the coding-agent loop or execute tools for this spec path.

The normal runtime boundary is direct Rust library use wherever Spark is already Rust-owned, especially `crates/spark-server`, `crates/spark-http`, `crates/spark-workspace`, and `crates/attractor-runtime`. Python compatibility facades may call a Rust-backed serialized process or FFI boundary only to preserve existing Python API tests and source-checkout compatibility; they are not the primary runtime implementation.

## Canonical Repository Topology

| Path | Ownership |
| --- | --- |
| `crates/spark-agent-adapter/src/lib.rs` | Public exports for the Rust agent runtime and Spark adapter DTOs. |
| `crates/spark-agent-adapter/src/agent.rs` | Spark-facing `AgentTurnRequest`, `AgentTurnOutput`, raw log, resume failure, and backend traits. These remain compatibility DTOs over the Rust session runtime. |
| `crates/spark-agent-adapter/src/session.rs` | `Session`, lifecycle state machine, submit/process loop, streaming loop, follow-up processing, abort, close, and event delivery. |
| `crates/spark-agent-adapter/src/config.rs` | `SessionConfig`, output limits, line limits, command timeout defaults, loop detection settings, and subagent depth. |
| `crates/spark-agent-adapter/src/history.rs` | `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SystemTurn`, and `SteeringTurn` with timestamps and conversion to unified LLM messages. |
| `crates/spark-agent-adapter/src/events.rs` | `SessionEvent`, `EventKind`, event emitter/channel support, stream-event conversion, and mapping into `spark_common::events::TurnStreamEvent`. |
| `crates/spark-agent-adapter/src/profiles/` | Provider-aligned `ProviderProfile` implementations for OpenAI, Anthropic, Gemini, and Spark compatibility profile extensions. |
| `crates/spark-agent-adapter/src/tools/` | `ToolDefinition`, `RegisteredTool`, `ToolRegistry`, argument validation, dispatch pipeline, built-in tool schemas, and provider-specific registrations. |
| `crates/spark-agent-adapter/src/environment.rs` | `ExecutionEnvironment`, `ExecResult`, `DirEntry`, `GrepOptions`, environment policy, and wrapper extension traits. |
| `crates/spark-agent-adapter/src/local_environment.rs` | Required local implementation for file operations, shell execution, grep, glob, metadata, initialize, cleanup, and process tracking. |
| `crates/spark-agent-adapter/src/apply_patch.rs` | OpenAI v4a patch parser and applier. Existing Python behavior may be used as an oracle, not as runtime delegation. |
| `crates/spark-agent-adapter/src/truncation.rs` | Character-first and line-second truncation with tool-specific defaults and visible markers. |
| `crates/spark-agent-adapter/src/context.rs` and `src/project_docs.rs` | Environment block, git snapshot, context-usage estimates, project instruction discovery, and prompt-layer inputs. |
| `crates/spark-agent-adapter/src/subagents.rs` | Subagent handles, status, spawn/send/wait/close tools, depth limits, and child cleanup. |
| `crates/spark-agent-adapter/src/llm_backend.rs` | Spark adapter implementations over the Rust session runtime, including chat turns and codergen-compatible LLM execution. Existing one-shot helpers can remain only as implementation details for text-only flows after they obey the session/event contract. |
| `crates/unified-llm-adapter` | Low-level LLM SDK boundary. The agent uses its public `Client`, `Request`, `Message`, `Tool`, `ToolCall`, `ToolResult`, `StreamEvent`, and `Usage` types directly. |
| `crates/spark-workspace/src/conversations.rs` | Conversation persistence, turn preparation, and ingestion of `AgentTurnOutput` events into existing Workspace conversation snapshots. |
| `crates/spark-http/src/workspace.rs` | HTTP routes for project conversation turn submission and request-user-input answers. Normal execution should connect prepared turns to the Rust agent backend without Python `src/agent`. |
| `crates/attractor-runtime` | Runtime handler wiring for codergen-adjacent execution. Codergen stages that require agent behavior use `spark-agent-adapter` rather than Python agent sessions. |
| `src/spark/chat/session.py` and `src/attractor/api/codex_backends.py` | Retained Python compatibility facades. They must route normal agent-backed behavior to the Rust boundary or be limited to compatibility/oracle status. |
| `tests/agent` | Existing Python oracle and compatibility suite for agent behavior. It does not prove Rust distribution readiness by itself. |
| `tests/compat/agent` and Rust crate tests | New or expanded behavior tests for Rust runtime parity and Spark facade compatibility. |
| `docs/rust-rewrite-migration.md` and `README.md` | Distribution guidance, runtime ownership, retained Python status, validation commands, provider configuration, event consumption, and explicit non-goals. |

## Implementation Boundaries

The Rust session loop owns the control plane:

1. append and emit user input;
2. drain queued steering;
3. build a low-level unified LLM `Request` with system prompt, history messages, provider-aligned tools, `tool_choice=auto`, provider id, provider options, and current reasoning effort;
4. call `Client.complete` or `Client.stream`;
5. record assistant text, reasoning, tool calls, usage, response id, and timestamp;
6. execute model-requested tools through `ToolRegistry` and `ExecutionEnvironment`;
7. append `ToolResultsTurn`, drain steering, check loop detection, and continue until a stop condition fires;
8. process queued follow-ups after natural completion;
9. return to `IDLE`, `AWAITING_INPUT`, or `CLOSED` and emit the corresponding lifecycle events.

The unified LLM high-level `generate` API is intentionally not used for this runtime because it owns its own tool loop. This implementation needs to interleave tool execution with truncation, steering, event emission, loop detection, Spark persistence, subagent lifecycle, timeouts, and shutdown cleanup.

Recoverable tool failures are data returned to the model as `ToolResult { is_error: true }`. Authentication failures, aborts, unrecoverable provider errors, and shutdown failures are session-level errors. Transient provider retry policy remains in the unified LLM adapter layer where already implemented; the agent loop does not retry whole multi-step operations after tool side effects.

## Public Interface

The documented Rust library surface is:

- `Session`, with creation, `process_input` or equivalent submit method, `steer`, `follow_up`, dynamic config updates, event stream access, abort, and close.
- `SessionConfig`, `SessionState`, and turn records: `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SystemTurn`, and `SteeringTurn`.
- `SessionEvent` and `EventKind`, with a timestamp, session id, and structured payload.
- `ProviderProfile`, including id, model, registry, `build_system_prompt`, `tools`, `provider_options`, capability flags, and context window size.
- `ToolDefinition`, `RegisteredTool`, `ToolRegistry`, and the tool dispatch helpers used by sessions and tests.
- `ExecutionEnvironment`, `LocalExecutionEnvironment`, `ExecResult`, `DirEntry`, `GrepOptions`, and environment policies.
- Built-in provider-aligned tool registration functions for OpenAI, Anthropic, Gemini, and Spark compatibility extensions.
- `SubAgentHandle`, `SubAgentStatus`, and `SubAgentResult` where these are exposed beyond internal tool execution.
- Spark adapter DTOs: `AgentTurnRequest`, `AgentTurnOutput`, `AgentRawLogLine`, `AgentThreadResumeFailure`, `AgentTurnBackend`, `CodergenBackendRequest`, `CodergenBackendOutput`, and `CodergenBackend`.

Rust type internals may be idiomatic, but serialized event, tool, request, response, usage, and Spark adapter payloads are public compatibility contracts. Existing `spark_common::events::TurnStreamEvent` names remain the Spark-facing event vocabulary; `SessionEvent` values are mapped into that vocabulary for Workspace persistence and live API consumers.

## Session Loop And Controls

`Session` is reusable across sequential user inputs until closed. Each processing cycle has its own `max_tool_rounds_per_input` counter while `max_turns` applies across the whole session history. `reasoning_effort` is read when constructing each LLM request so runtime changes take effect on the next model call.

`steer(message)` queues a `SteeringTurn` for injection before the next LLM call after the current tool round. If the session is idle, the message is retained until the next submit and drained before the first LLM call. Steering turns convert to user-role messages for provider request translation.

`follow_up(message)` queues a new user input to process after the current input naturally completes. Follow-ups preserve session history and event stream semantics, but they do not run after abort or unrecoverable close.

Stop conditions are natural completion, per-input round limit, session turn limit, abort, and unrecoverable error. Context usage above 80 percent of the provider window emits `WARNING`; it does not compact, summarize, or close the session.

## Event Translation

The canonical session stream includes every event in the spec plus the stream-derived events described by Section 1.6: assistant reasoning start/delta/end, model-proposed tool-call start/delta/end, and model usage updates. Model-proposed tool-call events describe what the LLM emitted; actual tool execution events remain `TOOL_CALL_START`, optional `TOOL_CALL_OUTPUT_DELTA`, and `TOOL_CALL_END`.

`TOOL_CALL_END` carries the full untruncated output or full error payload for hosts. The `ToolResult` sent back to the LLM carries truncated content when limits apply.

Spark-facing mapping preserves existing `TurnStreamEvent` behavior:

- assistant text maps to `content_delta` and `content_completed` on the assistant channel;
- reasoning maps to the reasoning channel;
- usage maps to `token_usage_updated`;
- tool start, update, complete, and failure map to existing tool-call event kinds and `ToolCallRecord`-compatible payloads;
- request-user-input questions remain visible through existing Workspace request input segments where supported;
- session errors map to `error` and preserve structured details for thread resume and failure reporting.

## Provider Profiles

Provider alignment is model-facing behavior, not an internal convenience. OpenAI, Anthropic, and Gemini profiles expose native tool names, schemas, editing formats, prompt topics, timeout defaults, and provider options.

OpenAI is codex-rs aligned: `apply_patch` is the primary editing path, `write_file` remains for full new-file writes, shell defaults to 10 seconds, and reasoning effort maps to OpenAI provider options through the unified LLM request.

Anthropic is Claude Code aligned: `edit_file` with `old_string` and `new_string` is the native editing path, `apply_patch` is not exposed as the native editing path, shell defaults to 120 seconds unless `timeout_ms` is set, and Anthropic beta headers pass through provider options.

Gemini is gemini-cli aligned: file, batch-read, edit, shell, grep, glob, list directory, optional web tools, and subagent tools are exposed with Gemini-style semantics. Gemini safety, grounding, and thinking settings pass through provider options.

OpenAI-compatible, OpenRouter, and LiteLLM selectors are Spark compatibility extensions over the unified LLM adapter. They can reuse an OpenAI-like profile only when that is explicit and documented; they do not count as native OpenAI, Anthropic, or Gemini parity.

Custom tool registration is latest-wins by name. Tests assert observable tool availability, schemas, provider options, and editing surface choices rather than byte-for-byte prompt strings.

## Tools And Execution

The dispatch pipeline is lookup, argument parsing, JSON Schema validation, execution, truncation, event emission, and `ToolResult` construction. Unknown tools, invalid arguments, and executor failures return error `ToolResult` values instead of closing the session.

Built-in tools use the active `ExecutionEnvironment` for file, command, grep, glob, and directory operations. They do not call the host filesystem directly except through the environment implementation. The OpenAI `apply_patch` parser and applier are Rust-owned and support add, delete, update, rename, multi-hunk, context matching, and end-of-file markers.

When a profile supports parallel tool calls, multiple tool calls from one assistant turn execute concurrently while preserving association by `tool_call_id` and preserving result order for the continuation turn.

## Execution Environment

`ExecutionEnvironment` is the only execution abstraction that tools depend on. The required local implementation resolves paths relative to `working_directory`, supports lifecycle initialize and cleanup, exposes platform and OS version, and tracks running command process groups so abort and timeout cleanup can terminate them.

Local shell execution uses `/bin/bash -c` on Linux and macOS and `cmd.exe /c` on Windows. Default timeout is 10000 ms, max timeout is 600000 ms, with the Anthropic shell profile default of 120000 ms unless the call sets `timeout_ms`. On timeout the runtime sends SIGTERM to the process group, waits 2 seconds, sends SIGKILL to remaining processes, and returns collected output plus the spec timeout message.

Environment filtering excludes names matching `*_API_KEY`, `*_SECRET`, `*_TOKEN`, `*_PASSWORD`, and `*_CREDENTIAL` case-insensitively by default. Policies support inherit all, inherit none, and core-safe variable inheritance. `grep` uses ripgrep when available and a Rust regex fallback otherwise.

Alternative environments such as Docker, Kubernetes, WASM, SSH, logging wrappers, and read-only wrappers implement the same trait without changing tool logic.

## Tool Output Management

Truncation is deterministic and runs before content goes back to the LLM. Character truncation always runs first and line truncation runs second where configured. Character limits and modes default to the spec table: `read_file` 50000 head_tail, `shell` 30000 head_tail, `grep` 20000 tail, `glob` 20000 tail, `edit_file` 10000 tail, `apply_patch` 10000 tail, `write_file` 1000 tail, and `spawn_agent` 20000 head_tail.

Line limits default to `shell=256`, `grep=200`, `glob=500`, and no line limit for read/edit outputs. Markers state what was omitted and that full output is available in the event stream. Truncation must operate on valid UTF-8 character boundaries.

## System Prompt And Project Context

System prompt construction is layered in this order: provider-specific base instructions, environment context, active tool descriptions, project-specific instructions, and user overrides. Provider base prompts cover the required topics but are not tested as fixed source strings.

The environment block is snapshotted at session start and includes working directory, git repository flag, branch, platform, OS version, date, model display name, and knowledge cutoff when known. Git context includes branch, short modified and untracked counts, and recent commit messages.

Project instruction discovery walks from git root, or working directory outside git, to the active working directory. `AGENTS.md` is universal. `CLAUDE.md`, `GEMINI.md`, and `.codex/instructions.md` load only for Anthropic, Gemini, and OpenAI respectively. Root-level files load first, deeper files load later, and the total instruction budget is 32 KB with the required truncation marker.

Approximate context usage uses the spec heuristic of one token per four characters and emits a `WARNING` above 80 percent of the active profile window. Automatic compaction remains out of scope.

## Subagents

Subagents are child `Session` values with independent histories and shared execution environment. They inherit the parent profile unless an allowed model override is supplied. The default maximum subagent depth is 1, which prevents recursive sub-sub-agent spawning.

`spawn_agent`, `send_input`, `wait`, and `close_agent` are ordinary registered tools. Their results are returned to the parent as `ToolResult` values and surfaced through the parent event stream. Parent abort or unrecoverable shutdown closes active subagents before the parent emits final `SESSION_END`.

## Spark Integration

Rust Workspace conversation turn preparation already owns persistence and live event ingestion through `PreparedConversationTurn` and `ingest_agent_turn_output`. The missing runtime execution boundary is a Rust agent backend that consumes prepared turn data, project path, provider, model, profile, reasoning effort, chat mode, conversation id, and persisted history, then returns `AgentTurnOutput` for ingestion.

Normal project chat and agent turns should therefore run:

1. HTTP route prepares a conversation turn in `crates/spark-workspace`;
2. Rust agent backend builds or resumes a `Session` scoped to the project path;
3. `Session` emits events while processing the turn;
4. events are mapped into `AgentTurnOutput` and ingested back into the conversation snapshot and live event journal.

Codergen-adjacent runtime paths in `crates/attractor-runtime` use `spark-agent-adapter` backends. Where a codergen stage needs a normal model answer, the Rust backend may produce a text-only session result. Where it needs agent behavior, it must use the same Rust session/tool/event boundary rather than Python `src/agent`.

Python `src/spark/chat/session.py` and `src/attractor/api/codex_backends.py` remain compatibility facades while tests and callers still import them. Their normal agent-backed behavior must call the Rust boundary or be documented as retained compatibility only; they must not continue to construct Python `agent.Session`, Python provider profiles, or Python `unified_llm.Client` for the distributed runtime.

## Error Handling And Shutdown

Tool-level errors are recoverable `ToolResult` values. This includes file not found, edit conflicts, nonzero shell exit, shell timeout, permission denied, validation error, and unknown tool.

Session-level errors affect lifecycle. Authentication errors and other non-retryable provider failures emit `ERROR`, run graceful cleanup, emit `SESSION_END`, and transition to `CLOSED`. Context length errors emit `WARNING` and continue when possible. Turn limits emit `TURN_LIMIT` and return to `IDLE`.

Abort or unrecoverable shutdown cancels in-flight LLM streams, terminates tracked command process groups, waits 2 seconds, kills remaining processes, flushes events, closes subagents, emits `SESSION_END`, and transitions to `CLOSED`.

## Validation Strategy

The repository validation command is `uv run pytest -q`. Failure triage uses `uv run pytest -q -x --maxfail=1 <path-or-nodeid>`. Rust `cargo test` commands are supplemental milestone evidence, especially for `crates/spark-agent-adapter` and `crates/unified-llm-adapter`, but final completion still requires the repository pytest command unless the user changes the policy.

Validation is behavioral:

- Rust unit and integration tests cover session lifecycle, event stream, request construction, streaming conversion, profile tools and options, registry behavior, tool errors, parallel tool execution, truncation, local environment, project docs, steering, follow-ups, loop detection, subagents, errors, and shutdown.
- Spark Workspace and HTTP tests cover prepared turn execution, event ingestion, live event envelopes, token usage, raw logs, thread resume failures, request-user-input records, and conversation state transitions.
- Codergen tests cover agent-backed execution, status envelope repair, write contract preservation, usage events, runtime failures, and artifact/log compatibility.
- Compatibility tests under `tests/compat/agent` exercise Python facades through real public APIs while proving that normal behavior is backed by Rust-owned output, not Python `src/agent`.
- Existing `tests/agent` may remain as Python oracle coverage. They are not sufficient completion evidence for this Rust runtime.
- Cross-provider parity covers OpenAI, Anthropic, and Gemini for file creation, read/edit, multi-file work, shell, timeout, grep, glob, multi-step tasks, truncation, parallel tool calls where supported, steering, reasoning effort change, subagents, loop detection, tool error recovery, and provider-native editing formats.
- Live smoke tests are optional, gated by credentials, and skipped in ordinary local validation.

Tests must assert observable behavior through APIs, event streams, files, tool results, and state transitions. They must not assert source text, prompt byte strings, deprecated implementation details, or workflow runtime aliases.

## Repository Hygiene

Implementation must stay in this Rust rewrite worktree. Do not add environment-specific hacks, test-only bootstrap behavior, shell wrappers around Python agent modules, or duplicate delivery layers that exist only to pass compatibility tests.

Python `src/agent` can stay tracked while compatibility tests and oracle references need it, but docs must identify it as retained compatibility or historical implementation once Rust owns normal runtime execution. `__pycache__`, build output, live API logs, local smoke artifacts, credential files, and generated runtime data are not deliverables.

Dependency additions must be justified by runtime need. Prefer existing workspace dependencies where they fit: `serde`, `serde_json`, `jsonschema`, `thiserror`, `time`, `tokio`, and existing HTTP/streaming utilities from `unified-llm-adapter`.

Documentation updates in `docs/rust-rewrite-migration.md` and `README.md` must state the Rust-owned agent runtime boundary, retained Python status, provider configuration, profile selection, reasoning controls, project instruction loading, validation commands, event consumption, steering/follow-up APIs, execution environment swapping, and out-of-scope extensions.

## Requirement Dependencies And Milestones

| Milestone | Requirement IDs | Dependency summary |
| --- | --- | --- |
| M1 Rust runtime boundary | REQ-001, REQ-002, REQ-003, REQ-004 | Establish Rust ownership, session lifecycle, core loop, low-level LLM request construction, and typed events. `REQ-003` depends on profile and tool foundations from M2, so M1 can land scaffolding before full behavior closes. |
| M2 provider tool execution | REQ-005, REQ-006, REQ-007, REQ-008 | Depends on the Rust boundary. Implements provider profiles, registry dispatch, built-in tools, and execution environment abstraction. |
| M3 context events control | REQ-009, REQ-010, REQ-011, REQ-012 | Depends on sessions, profiles, tools, and events. Implements truncation, prompt/project context, steering, reasoning controls, stop conditions, and loop detection. |
| M4 subagents shutdown | REQ-013, REQ-014 | Depends on session loop, profiles, environment, and events. Implements subagent tools and graceful error/abort shutdown. |
| M5 Spark integration validation | REQ-015, REQ-016, REQ-017 | Depends on the complete runtime. Wires Spark chat and codergen paths, completes behavior validation and cross-provider parity, and documents distribution readiness. |

Downstream implementation items must bind to the relevant decisions in `specs/coding-agent-rust-runtime/contract-decisions.json`. A material spec-versus-repository conflict must be resolved there before implementation depends on it.
