# Result

Implemented one-process-per-turn Claude Code control support, with structured stdin prompts, capability-gated channel retention, request-ID matching, existing question events/checkpoints/answer routing, and a minimal Claude Stop button and HTTP endpoint. Other permissions remain allowed; `bypassPermissions` is retained. Missing capabilities close stdin; malformed exchanges log a diagnostic, invalidate pending controls, and close stdin while normal output continues. A missing init also falls back after five seconds.

The installed CLI advertises `interrupt_receipt_v1`, `interrupt_cancel_queued_v1`, and `msg_lifecycle_v1`; it has no separately advertised question capability. This implementation uses `interrupt_receipt_v1` as the required compatibility gate for this turn-control surface and ignores unrelated capabilities.

Answers remain scoped to the live project/conversation and control request ID. Restarted or dead processes return `request_user_input_not_pending`; existing workspace logic expires the persisted question. Codex routing and segment schemas are unchanged. Interrupted partial text becomes the final answer using its existing item ID; an interrupt before text yields “Stopped.” Only an acknowledged interrupt with an explicit abort terminal reason suppresses the CLI's error flag; actual API/execution failures still fail.

## Validation

- Adapter contracts: question/answer and permissive tool-response wire shapes; request/conversation mismatch and duplicate answers; capability fallback; malformed JSON, requests, and responses; matched/mismatched interrupt receipts; ordinary and aborted completion; genuine errors after interrupt; answers after abrupt process death. Existing normalization and model-discovery contracts retained.
- Workspace contracts: durable pending checkpoint, answering through a newly opened service, resumed completion, dead-process expiration, interrupted text preservation, and normal shortened-turn completion. Existing Codex and transcript contracts retained.
- HTTP and frontend contracts: Stop dispatch reaches the backend without changing durable turn state; active Claude turns expose Stop; Codex behavior stays unchanged.
- Repository gate: **`just test` passed** on the final implementation: `cargo fmt --all -- --check`, `cargo test --workspace --all-features`, all **599 frontend unit tests** across 82 files, and `npm --prefix frontend run build`. Existing opt-in external-service tests remained ignored. The production build emitted its existing large-chunk warning.

## Installed CLI verification

Verified on 2026-09-23 with `/Users/chris/.local/bin/claude`, version **2.1.220**. A normal authenticated model probe returned “Not logged in · Please run /login.” To verify the actual installed executable without credentials, a temporary Python standard-library HTTP/SSE fixture supplied deterministic model responses over localhost using `ANTHROPIC_BASE_URL` and a fixture-only API key. This exercised the real CLI, its tool handling, permission responder, cancellation, and process exit; it did not test Anthropic service authentication or real model behavior.

Invocation used `-p --input-format stream-json --output-format stream-json --include-partial-messages --verbose --permission-mode bypassPermissions --permission-prompt-tool stdio --no-session-persistence --setting-sources '' --model claude-sonnet-4-6`. No initialization control request was needed. The separate model-discovery probe was not changed.

Observed exchanges (dynamic IDs abbreviated):

```json
{"type":"user","message":{"role":"user","content":"Ask which color."}}
{"type":"control_request","request_id":"question-id","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","display_name":"AskUserQuestion","input":{"questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"Warm"},{"label":"Blue","description":"Cool"}],"multiSelect":false}]},"tool_use_id":"toolu_probe","requires_user_interaction":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"question-id","response":{"behavior":"allow","updatedInput":{"questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"Warm"},{"label":"Blue","description":"Cool"}],"multiSelect":false}],"answers":{"Which color?":"Blue"}}}}}
```

Question verification **passed**: the next API request contained the CLI-generated tool result `Your questions have been answered: "Which color?"="Blue"...`; the CLI emitted `result`, `subtype: success`, `is_error: false`, text “Blue confirmed.”, and exited 0. This confirms that answer keys are the original question text, and that `bypassPermissions` does not bypass `AskUserQuestion`.

Interrupt verification **passed** during a streaming response and while a question was pending:

```json
{"type":"control_request","request_id":"spark-probe-interrupt","request":{"subtype":"interrupt"}}
{"type":"control_response","response":{"subtype":"success","request_id":"spark-probe-interrupt","response":{"still_queued":[]}}}
```

For a pending question, the CLI first emitted `{"type":"control_cancel_request","request_id":"question-id"}`, then the receipt and a user-rejected tool result. Both interrupts finished through `result` and exit 1: `subtype: error_during_execution`, `is_error: true`, and `terminal_reason: aborted_streaming` or `aborted_tools`. These real shapes are pinned in the fake and normalize to successful shortened turns. An unauthenticated interrupt probe also confirmed the same receipt shape, but did not establish model-turn completion.

Excluded scope remains excluded: cross-turn process reuse, permission policy/UI, queued or steering messages, stop polish, new segment kinds, and model discovery changes. Multi-select questions use the existing free-text answer field with displayed choices and comma-separated answers; a dedicated multi-select UI is deferred until segment/UI support is requested.
