# Speak The Claude Code Bidirectional Control Protocol

## Summary
The claude_code adapter drives the CLI as a one-shot pipe: prompt over stdin, stdin closed, events drained until exit. That forecloses every mid-turn interaction the protocol supports — the CLI can ask the user structured questions (`AskUserQuestion`), request permission decisions (`can_use_tool`), and accept interrupts, all over the same stream-json channel the official Agent SDKs use by holding stdin open (`--input-format stream-json`) and exchanging `control_request`/`control_response` messages. Codex conversations already have mid-turn `request_user_input` in spark; claude-code conversations cannot answer a question or be stopped without killing the process.

Upgrade the adapter to hold the turn's stdin open and speak the control protocol, wiring `AskUserQuestion` into spark's existing request-user-input machinery and exposing interrupt. Keep the process-per-turn lifecycle: the interactive features are per-turn properties, and cross-turn process reuse is a latency optimization with its own lifecycle burden — explicitly deferred.

The wire protocol is the Agent SDK's internal surface, not formally documented. Mitigations are mandatory, not optional: bidirectional mode activates only when the CLI's `system/init` `capabilities` array (Claude Code v2.1.205+) advertises the needed behaviors; any control-protocol failure mid-turn degrades the turn to today's fire-and-forget semantics instead of failing it; and the fake CLI pins the learned wire shapes as the contract, per repo practice.

## Key Changes
- Hold the turn channel open and route control traffic (`crates/spark-agent-adapter/src/claude_code.rs`).
  - Spawn with `--input-format stream-json`; write the prompt as a stream-json user message and keep stdin open for the life of the turn.
  - Parse `system/init` and record the `capabilities` array. When required capabilities are absent (older CLI), close stdin after the prompt and behave exactly as today — the fallback is the current behavior, not an error.
  - Handle CLI→spark `control_request` messages and reply with `control_response` over stdin. Derive exact wire shapes by probing the installed CLI; encode every learned shape into the fake CLI so the contract is pinned.
  - On any malformed or unexpected control exchange: log it into the raw log lines, stop responding to control traffic, and let the turn complete in degraded mode.
- Route `AskUserQuestion` through spark's existing request-user-input flow.
  - When a `can_use_tool`/control request carries an `AskUserQuestion` invocation, emit a `RequestUserInputRequested` event (the backend-agnostic kind codex already uses) so the workspace persists its pending segment and checkpoint-commits, exactly as for codex.
  - Deliver the user's answers back as the `control_response`, resuming the still-running turn. Keep a live pending-request registry keyed by request id, mirroring the codex adapter's; an answer arriving after the turn process has died produces the same "no longer pending" resume-failure message codex produces.
  - Workspace dispatch (`crates/spark-workspace/src/conversations.rs`): the answer path routes to the claude_code backend when the conversation's provider is claude-code.
- Answer all other permission requests with allow.
  - The default responder preserves today's effective behavior (the adapter currently runs `bypassPermissions`): every non-AskUserQuestion permission request is approved. If enabling the control channel requires dropping `--permission-mode bypassPermissions`, the allow-everything responder keeps observable behavior identical. Permission policy (ask rules, deny rules, approval UX) is explicitly out of scope.
- Expose interrupt.
  - Add an interrupt handle to the running turn (spark→CLI `control_request`), gated on the `interrupt_receipt_v1` capability. An interrupted turn winds down through the CLI's own result path and finalizes as a normal (shortened) turn, not an error.
  - Workspace: expose interrupt on the active conversation turn so the frontend can wire a stop control. Minimal UI (a stop affordance on the streaming turn) is in scope; polish is not.
- Extend the fake CLI (`crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs`) with bidirectional scenarios: capability-advertising `system/init`; a mid-turn `AskUserQuestion` control request that blocks until the matching `control_response` arrives, then continues to a result that reflects the answer; an interrupt scenario that winds down with a result on receipt; a legacy scenario (no capabilities) exercising the fallback.

## Explicitly Out Of Scope
- Cross-turn process reuse (warm sessions): process-per-turn stays; follow-up CR if cold-start latency matters.
- Permission policy and approval UX: the responder allows everything except AskUserQuestion routing.
- Mid-turn user-message queuing/steering.
- The model-discovery probe: unchanged, separate short-lived process.

## Test Plan
- Adapter contract tests (`crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs`):
  - AskUserQuestion round-trip: turn emits `RequestUserInputRequested` with the question payload, blocks, resumes on delivered answers, and the final text reflects the answer.
  - Interrupt: interrupt handle mid-stream yields a completed (not failed) turn output whose events end at the wind-down result.
  - Capability fallback: the no-capabilities fake CLI scenario produces byte-identical behavior to the current one-shot mode (stdin closed after prompt, no control traffic).
  - Degraded mode: a malformed control request is logged and the turn still completes.
  - Existing normalization contracts (item ids, phases, deltas, tool payloads) pass unchanged in bidirectional mode.
- Workspace contract tests (`crates/spark-workspace`):
  - A claude-code turn with a pending request-user-input persists the pending segment via the checkpoint commit and resumes on answer, mirroring the codex flow.
  - An answer against a dead turn process yields the codex-parity "no longer pending" resume failure.
  - Codex request-user-input behavior is unchanged.
- Frontend: stop-control wiring has a minimal unit test; existing transcript tests pass unchanged.
- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- The control-protocol wire shapes are learned from the installed CLI and pinned by the fake CLI contract; the capabilities array is the activation gate, so a CLI that predates or changes the protocol degrades to current behavior rather than breaking turns.
- Effective permission behavior is unchanged by this CR (allow-everything responder ≙ today's bypassPermissions).
- The request-user-input answer path reuses the existing workspace machinery and segment shapes; no new segment kinds.
- A manual end-to-end smoke against the real installed CLI (ask a question that triggers AskUserQuestion; interrupt a running turn) is performed as part of validation and noted in result.md, since `just test` exercises only the fake CLI.
