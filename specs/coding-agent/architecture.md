# Coding Agent Loop Implementation Architecture

## Source of Truth

This architecture implements `.spark/spec-implementation/current/spec/source.md` and the extracted requirement ledger in `.spark/spec-implementation/current/spec/requirements.json`. The staged source document is the product behavior and external contract for this run.

The repository already contains a Python `src` layout package named `unified_llm`, including the low-level `Client.complete()` and `Client.stream()` APIs that the coding agent must use. The agent implementation is a top-level `agent` package under `src/agent/` in the same distribution, layered on `unified_llm`; it does not introduce a required CLI, a second distribution, or a wrapper-only delivery layer.

## Canonical Repository Topology

The implementation should converge on this topology:

```text
pyproject.toml
src/
  agent/
    __init__.py
    apply_patch.py
    builtin_tools.py
    context.py
    environment.py
    errors.py
    events.py
    history.py
    local_environment.py
    loop_detection.py
    profiles/
      __init__.py
      anthropic.py
      base.py
      gemini.py
      openai.py
    project_docs.py
    prompts.py
    session.py
    subagents.py
    tool_execution.py
    tools.py
    truncation.py
    types.py
  unified_llm/
    __init__.py
    client.py
    errors.py
    tools.py
    types.py
tests/
  agent/
    test_agent_loop.py
    test_anthropic_profile.py
    test_apply_patch.py
    test_builtin_tools.py
    test_context_usage.py
    test_cross_provider_parity.py
    test_environment.py
    test_error_handling.py
    test_events.py
    test_gemini_profile.py
    test_live_smoke.py
    test_local_environment.py
    test_loop_detection.py
    test_openai_profile.py
    test_project_docs.py
    test_prompts.py
    test_provider_profiles.py
    test_reasoning_effort.py
    test_session.py
    test_shutdown.py
    test_steering.py
    test_subagents.py
    test_tool_execution.py
    test_tool_registry.py
    test_truncation.py
```

`pyproject.toml` remains the package, dependency, pytest, and ruff configuration authority. Runtime dependencies should stay small: the current `jsonschema` dependency is the validation mechanism for tool schemas, and no new runtime dependency should be added unless it removes meaningful implementation risk. Development validation remains `uv run pytest -q` and `uv run ruff check .`.

## Implementation Boundaries

The agent package has five boundaries.

Layer 1, agent data model and events:

- `agent/types.py` defines `SessionConfig`, `SessionState`, `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SystemTurn`, `SteeringTurn`, and related agent records using dataclasses/enums consistent with the existing SDK style. `SessionState` includes `IDLE`, `PROCESSING`, `AWAITING_INPUT`, and `CLOSED`; the waiting state is used only when a text-only assistant response is classified as an open-ended request for user input.
- `agent/events.py` defines `EventKind`, `SessionEvent`, and an async event emitter or queue-backed event stream. It is the only layer responsible for delivering typed session events to hosts.
- `EventKind` is the full public event surface from `REQ-003` and includes `SESSION_START`, `SESSION_END`, `USER_INPUT`, `PROCESSING_END`, `ASSISTANT_TEXT_START`, `ASSISTANT_TEXT_DELTA`, `ASSISTANT_TEXT_END`, `TOOL_CALL_START`, `TOOL_CALL_OUTPUT_DELTA`, `TOOL_CALL_END`, `STEERING_INJECTED`, `TURN_LIMIT`, `LOOP_DETECTION`, `WARNING`, and `ERROR`.
- Agent events are distinct from SDK `StreamEvent` values. SDK stream events are consumed by the session loop and normalized into agent `SessionEvent` values for host observation.

Layer 2, execution environments:

- `agent/environment.py` defines the `ExecutionEnvironment` protocol plus `ExecResult`, `DirEntry`, `GrepOptions`, and environment-variable inheritance policy records.
- `agent/local_environment.py` implements the required local environment for filesystem access, shell execution, search, globbing, lifecycle hooks, metadata, process tracking, timeout enforcement, and environment filtering.
- Tool implementations depend only on the `ExecutionEnvironment` protocol. Docker, Kubernetes, WASM, SSH, sandboxed, logging, or read-only environments are extension points and are not required concrete implementations for this run.

Layer 3, tools and tool execution:

- `agent/tools.py` owns agent-level `ToolDefinition`, `RegisteredTool`, and `ToolRegistry`. It converts profile tool definitions into objects accepted by the existing SDK `Request.tools` field.
- `agent/tool_execution.py` owns lookup, JSON Schema validation, argument parsing, executor invocation, parallel execution, event emission, output truncation, result ordering, logging, and error-result conversion.
- `agent/builtin_tools.py` owns shared observable behavior for `read_file`, `write_file`, `edit_file`, `shell`, `grep`, `glob`, `read_many_files`, `list_dir`, and subagent tool registration helpers where appropriate.
- `agent/apply_patch.py` owns OpenAI's v4a patch parser and applier. It is a tool implementation detail, not a general-purpose file editing layer for every provider.
- `agent/truncation.py` and `agent/context.py` own model-facing output truncation and context-window warning calculations. Full tool output remains in `TOOL_CALL_END` event payloads.

Layer 4, provider profiles and prompts:

- `agent/profiles/base.py` defines `ProviderProfile` with id, model, tool registry, prompt construction, provider options, capability flags, context-window size, and profile hooks for provider-native completion classification when needed.
- `agent/profiles/openai.py`, `anthropic.py`, and `gemini.py` construct provider-aligned profiles and model-facing tool schemas while reusing shared tool behavior internally.
- `agent/prompts.py` assembles layered prompts from provider base instructions, environment context, tool descriptions, project documents, and user instruction overrides.
- `agent/project_docs.py` discovers `AGENTS.md`, `.codex/instructions.md`, `CLAUDE.md`, and `GEMINI.md` according to the active profile and 32KB budget.

Layer 5, orchestration:

- `agent/session.py` owns `Session`, async `process_input`, optional `submit` convenience with the same semantics, lifecycle transitions, history mutation, request construction, low-level SDK calls, streaming support, limits, steering, follow-ups, loop detection, context warnings, abort, and shutdown.
- `agent/history.py` converts agent turns to SDK `Message` values and preserves steering as user-role input.
- `agent/loop_detection.py` computes stable tool-call signatures and detects repeating patterns.
- `agent/subagents.py` owns child session handles, status, result records, task scheduling, cleanup, and the implementation behind `spawn_agent`, `send_input`, `wait`, and `close_agent`.
- `agent/errors.py` contains agent-specific error types only when existing SDK errors are not semantically precise enough. It should not duplicate the SDK provider error hierarchy.

## Documented Public Interface

The stable import surface is the top-level `agent` package, layered on the
existing `unified_llm` SDK package. `src/agent/__init__.py` should re-export:

- Session API: `Session`, `SessionConfig`, `SessionState`, turn records, `create_session` if a small factory proves useful.
- Events: `EventKind`, `SessionEvent`, and the public event stream interface. `EventKind` exports every host-visible kind required by the spec: `SESSION_START`, `SESSION_END`, `USER_INPUT`, `PROCESSING_END`, `ASSISTANT_TEXT_START`, `ASSISTANT_TEXT_DELTA`, `ASSISTANT_TEXT_END`, `TOOL_CALL_START`, `TOOL_CALL_OUTPUT_DELTA`, `TOOL_CALL_END`, `STEERING_INJECTED`, `TURN_LIMIT`, `LOOP_DETECTION`, `WARNING`, and `ERROR`.
- Profiles: `ProviderProfile`, `create_openai_profile`, `create_anthropic_profile`, `create_gemini_profile`.
- Environments: `ExecutionEnvironment`, `LocalExecutionEnvironment`, `ExecResult`, `DirEntry`, `GrepOptions`, and environment inheritance policy types.
- Tools: `ToolDefinition`, `RegisteredTool`, `ToolRegistry`, tool execution helpers that are intentionally public, and built-in tool registration functions.
- Subagents: `SubAgentStatus`, `SubAgentHandle`, `SubAgentResult`.
- Errors that callers can reasonably catch.

Canonical usage is async:

```python
from unified_llm import Client
from agent import (
    LocalExecutionEnvironment,
    Session,
    create_openai_profile,
)

client = Client(...)
profile = create_openai_profile(model="gpt-5.2-codex")
environment = LocalExecutionEnvironment(working_dir=".")
session = Session(profile=profile, execution_env=environment, llm_client=client)

async for event in session.events():
    ...

await session.process_input("Fix the failing tests")
```

`Session.process_input()` is the canonical implementation entrypoint for a user input. It is valid when the session is `IDLE` or `AWAITING_INPUT`; when called from `AWAITING_INPUT`, the input is recorded as the user's answer to the pending assistant question and resumes the same history. A `submit()` method may be exposed to match the spec's host terminology only if it shares the same implementation path and public semantics.

The agent builds SDK `Request` objects with `Message.system(...)`, converted history, active profile tools, automatic tool choice, `SessionConfig.reasoning_effort`, the profile provider id, and profile provider options. It calls `await Client.complete(request)` for non-streaming operation and consumes `Client.stream(request)` for streaming operation. It does not call the SDK high-level `generate()` tool loop.

The public contract is library-first. A CLI may be implemented later as a host application, but it must consume this API rather than own the core behavior.

## Provider Profile Contracts

Provider profiles own the model-facing surface. Shared tool functions may be reused internally, but the tool names, schemas, timeouts, prompt guidance, and provider options exposed to the model must stay provider-aligned:

- OpenAI profile: codex-rs-aligned tools with `apply_patch` as the primary edit mechanism, `read_file`, `write_file`, `shell`, `grep`, `glob`, and subagent tools. The shell default is 10000 ms. Provider options map configured reasoning effort into OpenAI `reasoning.effort` options in addition to the SDK `Request.reasoning_effort` field.
- Anthropic profile: Claude Code-aligned tools with `edit_file` old_string/new_string as the native edit mechanism, `read_file`, `write_file`, `shell`, `grep`, `glob`, and subagent tools. The shell default is 120000 ms while preserving the max timeout bound. Anthropic `grep` exposes the profile-native output modes `content`, `files_with_matches`, and `count`. Provider options can pass Anthropic beta headers and any explicitly configured native thinking options without inventing hidden defaults.
- Gemini profile: gemini-cli-aligned tools with `read_file`, `read_many_files`, `write_file`, Gemini-style `edit_file`, `shell`, `grep`, `glob`, `list_dir`, subagent tools, and explicitly configured optional `web_search` and `web_fetch`. Provider options can pass Gemini safety, grounding, thinking, and request options.

The architecture resolves the source document's reference-agent language as behavioral alignment for this repo. Prompts and tool definitions should cover the specified topics and model-facing affordances, but tests must not require brittle byte-for-byte prompt text from external CLIs.

Custom tool registration is latest-wins. If a host registers a tool with the same name as a built-in profile tool, the custom registration replaces the previous tool for that profile.

`read_file` image handling is profile-gated. Built-in read behavior should detect binary image files and return image data only when the active provider profile advertises multimodal file output support through its capability flags and the SDK boundary can represent that content. Profiles without that capability return a recoverable binary-file tool error rather than trying to coerce image bytes into text.

Subagent tools are part of the completed provider profile contract, not a schema-only placeholder. The common subagent runtime from `REQ-016` must be implemented before `REQ-010`, `REQ-011`, or `REQ-012` are considered complete. Profile modules may share registration helpers for `spawn_agent`, `send_input`, `wait`, and `close_agent`, but final profile validation must prove those exposed tools reach working child-session behavior through the normal tool execution path.

## Agent Loop Semantics

The session loop follows the source pseudocode with Python async mechanics:

- On input from `IDLE`, transition to `PROCESSING`, append `UserTurn`, emit `USER_INPUT`, and drain queued steering before the first model call.
- On input from `AWAITING_INPUT`, clear the pending question marker, transition back to `PROCESSING`, append the answer as a `UserTurn`, emit `USER_INPUT` with answer metadata, and continue with the existing history.
- Before each model call, enforce `max_tool_rounds_per_input`, `max_turns`, abort state, and context warning checks.
- Build a low-level SDK `Request` from the provider profile and converted session history.
- Record assistant text, reasoning, usage, response id, and tool calls in an `AssistantTurn`.
- Emit assistant text lifecycle events. Streaming responses emit start/delta/end events; non-streaming responses still emit a coherent text lifecycle.
- If no tool calls are present, classify the text-only response before completing. The default classifier is deterministic and conservative: a stripped assistant response whose final non-whitespace character is `?` is treated as an open-ended user question; provider profiles may supply a stricter classifier, but they must not make ordinary tool-free completions impossible to reach.
- For an open-ended user question, transition to `AWAITING_INPUT`, preserve history, store the assistant question for host inspection, and pause without emitting `PROCESSING_END` or processing queued follow-ups.
- For a non-question text-only response, naturally complete the current input.
- Execute tool calls through the profile's registry. If the profile supports parallel tool calls, execute concurrently and preserve original result order.
- Append a `ToolResultsTurn`, drain steering injected during tool execution, run loop detection when enabled, and continue.
- After natural completion, process queued follow-ups as new inputs before returning to `IDLE` and emitting `PROCESSING_END`. Follow-ups remain queued while the session is `AWAITING_INPUT`.

Turn limits stop the processing cycle by emitting `TURN_LIMIT` and returning the session to `IDLE` with accumulated history preserved. Abort or unrecoverable errors follow the graceful shutdown contract and transition to `CLOSED`.

## Tool Execution and Output Contract

Every tool call passes through the same pipeline:

```text
lookup -> validate -> execute -> truncate for model -> emit full output -> return ToolResult
```

Unknown tools, invalid arguments, permission failures, file-not-found errors, edit conflicts, nonzero shell exits, shell timeouts, and executor exceptions are recoverable tool-level failures. They become SDK `ToolResult` values with `is_error=True` and are logged when useful for diagnostics.

Tool execution emits `TOOL_CALL_START` before execution, emits `TOOL_CALL_OUTPUT_DELTA` when an execution environment or streaming tool can surface incremental output, and emits exactly one `TOOL_CALL_END` with full output or full error details for each call.

Shared tools should implement all behavior once and let profile wrappers adapt schemas and formatting. This includes Anthropic `grep` output modes, Gemini `read_many_files` and `list_dir` adapters, provider-specific shell timeout defaults, and image-read capability gating.

Truncation is model-facing only:

- Character truncation always runs first.
- Line truncation runs second for tools with configured line limits.
- `TOOL_CALL_END` events carry full untruncated output or full error information.
- `ToolResult.content` receives the truncated output and explicit truncation marker when a limit is exceeded.

`LocalExecutionEnvironment` resolves paths relative to its working directory, creates parent directories for writes, uses process groups where supported, applies the configured environment inheritance policy, filters sensitive environment variables by default, enforces the timeout sequence from the spec, uses ripgrep when available for grep, falls back to Python regex search, and sorts glob results by newest modification time first.

## System Prompt and Project Context

System prompts are assembled in this order:

1. Provider-specific base instructions.
2. Environment context.
3. Active tool descriptions.
4. Project-specific instruction documents.
5. User instruction overrides.

The environment context is snapshotted at session start and includes working directory, git repository boolean, branch, short modified/untracked counts, recent commit messages, platform, OS version, today's date, model display name, and knowledge cutoff when known. The agent should not embed full diffs in the prompt.

Project document discovery walks from git root or working directory to the active working directory. `AGENTS.md` is always loaded. `.codex/instructions.md` is OpenAI-only, `CLAUDE.md` is Anthropic-only, and `GEMINI.md` is Gemini-only. Root-level files are loaded first, deeper files later, and the total instruction budget is 32KB with the specified truncation marker.

Prompt tests should validate structure, ordering, included data, and provider-specific document filtering. They must not assert entire prompt strings.

## Error Handling and Shutdown

The agent uses standard library logging with module-level loggers and no direct printing. Expected recoverable tool failures become error results for the model. Unexpected exceptions are logged at an appropriate level before conversion or propagation.

Retry ownership stays with the Unified LLM SDK layer for transient provider errors. The agent loop does not reimplement provider retry policy. Authentication errors emit `ERROR`, surface to the caller, and close the session. Context-length errors emit `WARNING` and do not trigger automatic compaction.

Graceful shutdown owns all agent resources:

1. Cancel any in-flight LLM stream or pending model call when supported by the underlying iterator/client.
2. Terminate running local command process groups.
3. Wait 2 seconds, then kill remaining processes where supported.
4. Close active subagents.
5. Flush pending events.
6. Emit `SESSION_END` with final state.
7. Transition to `CLOSED`.

## Validation Strategy

The deterministic gates are:

```text
uv run pytest -q
uv run ruff check .
```

Failure triage should use:

```text
uv run pytest -q -x --maxfail=1 <path-or-nodeid>
```

Tests must exercise observable behavior through public APIs: session states, emitted events, SDK requests passed to fake clients, SDK tool results, filesystem effects, command output, search results, prompt structure, and subagent outcomes. Tests must not depend on source text, prompt prose, documentation strings, or private implementation details.

Event tests must cover the complete `REQ-003` surface, including lifecycle events (`SESSION_START`, `SESSION_END`, `PROCESSING_END`), assistant text events, tool events including `TOOL_CALL_OUTPUT_DELTA` and full-output `TOOL_CALL_END`, steering, limits, loop detection, `WARNING`, and `ERROR`.

Default tests are deterministic and require no provider credentials, live network access, Docker, Kubernetes, SSH, or OS sandbox availability. Live smoke tests are optional, explicitly marked or skipped by default, and run only when selected and required API keys are present.

The cross-provider parity suite should cover simple file creation, read-then-edit, multi-file edits, shell execution, timeout handling, grep/glob, multi-step read/analyze/edit tasks, large-output truncation, supported parallel tool calls, steering, reasoning-effort changes, subagent spawn/wait, loop detection, tool error recovery, and provider-specific edit formats.

Before reporting completion of code changes in this repository, run `uv run pytest -q` as required by `AGENTS.md`. When agent implementation milestones add lintable code, also run `uv run ruff check .` for the spec's lint gate.

## Repository Hygiene Expectations

- Keep implementation in the top-level `agent` package layered on `unified_llm`, with tests under `tests/agent/`.
- Do not add a mandatory CLI or wrapper layer for this core library work.
- Do not use test-only bootstrap behavior, environment-specific hacks, or compatibility shims as the primary runtime path.
- Do not commit API keys, credentials, local absolute-path assumptions, live-provider outputs, or network-dependent default tests.
- Keep profile-specific behavior in profile modules and shared behavior in helpers to avoid drift.
- Preserve provider-native model-facing affordances rather than forcing all profiles into one universal edit or shell schema.
- Use `jsonschema` for JSON Schema validation instead of ad hoc argument validation when practical.
- Use standard library `logging`; no library `print()` calls.
- Handle or log exceptions according to whether they are tool-level recoverable, session-level terminal, or caller-visible SDK errors.
- Keep tests behavior-first and resilient to harmless refactors or prompt rewording.

## Requirement Dependencies and Milestone Flow

The milestone flow is dependency-ordered. A worker must not mark a requirement complete before its listed dependencies have passed, even when a preliminary milestone label would otherwise place the dependent requirement earlier.

Milestone M1 establishes the public library foundation:

- `REQ-001` creates the importable agent package and SDK integration boundary.
- `REQ-002` depends on `REQ-001` for session, config, lifecycle, and turn records.
- `REQ-003` depends on `REQ-002` for the typed session event stream.

Milestone M2 builds execution infrastructure:

- `REQ-007` depends on `REQ-002` for execution environments and local command/filesystem behavior.
- `REQ-008` depends on `REQ-002` and `REQ-003` for truncation and context usage warnings.
- `REQ-005` depends on `REQ-003`, `REQ-007`, and `REQ-008` for the registry and tool execution pipeline.
- `REQ-009` depends on `REQ-005` for provider profiles and custom tool extensibility.

Milestone M3 builds orchestration:

- `REQ-013` depends on `REQ-007` and `REQ-009` for layered prompts and project context.
- `REQ-014` depends on `REQ-002` and `REQ-003` for steering, follow-ups, and reasoning effort.
- `REQ-015` depends on `REQ-002`, `REQ-003`, and `REQ-005` for loop detection.
- `REQ-004` depends on `REQ-002`, `REQ-003`, `REQ-005`, `REQ-009`, `REQ-013`, and `REQ-014` for the full processing loop.

Milestone M4 completes shared tools and the subagent foundation:

- `REQ-006` depends on `REQ-005` and `REQ-007` for shared file, shell, grep, and glob behavior.
- `REQ-016` depends on `REQ-004`, `REQ-005`, and `REQ-007` for child sessions, depth limits, shared execution environments, subagent handles, result records, task scheduling, and common tool executors behind `spawn_agent`, `send_input`, `wait`, and `close_agent`.

Milestone M5 completes provider profiles and shutdown:

- `REQ-010`, `REQ-011`, and `REQ-012` depend on `REQ-006`, `REQ-009`, and `REQ-016` for OpenAI, Anthropic, and Gemini profiles. Profile-specific tests may validate tool names, schemas, prompts, provider options, edit mechanisms, and timeout defaults independently, but no provider profile requirement is complete until its subagent tools execute end to end through the working `REQ-016` runtime.
- `REQ-017` depends on `REQ-003`, `REQ-004`, `REQ-005`, `REQ-007`, and `REQ-016` for session error handling and graceful shutdown.

Milestone M6 validates completion:

- `REQ-018` depends on `REQ-010`, `REQ-011`, `REQ-012`, and `REQ-017` for deterministic cross-provider parity and optional live smoke validation.
