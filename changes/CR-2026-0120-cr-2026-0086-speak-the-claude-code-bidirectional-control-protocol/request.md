# CR-2026-0086: Speak the Claude Code Bidirectional Control Protocol

Restated from the existing [change request](/Users/chris/projects/spark/changes/CR-2026-0086-claude-code-bidirectional-control-protocol/request.md) for review, preserving its scope and assumptions.

## Summary

Enable Claude Code conversations to ask structured questions mid-turn, receive answers, and accept interrupts through the CLI’s stream-json control protocol. Retain one process per turn.

## Implementation changes

- **Keep the turn channel open.** Start the CLI with `--input-format stream-json`, send the prompt as a user message, and retain stdin for control responses. Activate bidirectional behavior only when `system/init` advertises the required capabilities. Otherwise, close stdin after the prompt and preserve existing one-shot behavior.
- **Handle control messages.** Receive `control_request` messages and send matching `control_response` messages. Learn exact wire shapes from the installed CLI and pin them in the fake CLI. Log malformed or unexpected exchanges, stop responding to control traffic, and allow the turn to finish in degraded mode.
- **Connect `AskUserQuestion` to existing question handling.** Emit `RequestUserInputRequested`, persist the pending segment through the existing checkpoint flow, and return the user’s answers over stdin. Maintain pending requests by request ID. Answers submitted after the process dies receive the existing “no longer pending” resume failure.
- **Route workspace answers to the correct backend.** Dispatch Claude Code answers to its adapter while preserving Codex behavior and existing segment shapes.
- **Preserve permission behavior.** Allow every permission request other than `AskUserQuestion`, matching the adapter’s existing `bypassPermissions` behavior. If the control channel requires removing that flag, retain equivalent behavior through the responder.
- **Expose interrupt and a minimal stop control.** Gate interrupt requests on `interrupt_receipt_v1`. Let the CLI wind down through its normal result path, completing the shortened turn without marking it failed.
- **Extend the fake CLI.** Cover capability advertisement, a question that blocks until answered, interrupt followed by a normal result, legacy fallback, and malformed control traffic.

## Validation

- Adapter contracts verify question/answer round trips, normal completion after interrupt, legacy fallback, and successful completion after malformed control traffic.
- Existing normalization contracts for item IDs, phases, deltas, and tool payloads continue to pass.
- Workspace contracts verify pending-question persistence, resumption, dead-process answer failures, and unchanged Codex behavior.
- Add a minimal stop-control wiring test; retain existing transcript tests.
- Run:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`
- Manually verify a question/answer exchange and interrupt against the installed Claude Code CLI; record results in `result.md`.

## Assumptions and exclusions

The original CR assumes the CLI advertises the necessary capabilities through `system/init` and that control-protocol failures can degrade to existing behavior. Exact protocol shapes must be established against the installed CLI and captured in fake-CLI contracts.

Reuse existing request-user-input events and segments; introduce no new segment kinds.

Exclude cross-turn process reuse, permission policy and approval UI, mid-turn message queuing or steering, stop-control polish, and changes to the separate model-discovery probe.
