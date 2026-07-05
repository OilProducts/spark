# Block Codex App-Server Request-User-Input Until User Answer

## Summary
Make Rust match the Python behavior for `item/tool/requestUserInput`: when Codex asks a question, Spark should emit a pending request card and hold the app-server JSON-RPC request open until the user submits an answer. It must not auto-answer with empty answers or continue inference before the user responds.

## Key Changes
- Replace the Rust empty-answer path in the Codex app-server client with a pending-answer bridge:
  - On `item/tool/requestUserInput`, normalize and emit the existing `request_user_input_requested` event.
  - Register the pending JSON-RPC request by request id/question ids.
  - Block the active app-server read loop until an answer is submitted, then send the JSON-RPC response shaped as `{ "answers": { "<question_id>": { "answers": ["..."] } } }`.
  - Keep command/file approval behavior unchanged.

- Wire workspace answer submission into the live pending app-server request:
  - Reuse the existing `/request-user-input/{request_id}/answer` route and `submit_request_user_input_answer` flow.
  - Change the Codex backend’s `answer_request_user_input` implementation so it delivers answers to the pending live app-server request instead of using `turn/steer`.
  - Keep the existing expired behavior when there is no live pending request, the request id mismatches, the request is already answered, or the app-server process has gone away.

- Preserve current persisted UI behavior:
  - The request segment is persisted as `pending` when the question appears.
  - On user submit, the segment becomes `answered` with submitted answers and `submitted_at`.
  - The assistant turn remains streaming/pending while waiting and completes only after Codex continues and returns final output.
  - Do not create a plan/final assistant segment from the same turn before the answer is accepted.

## Test Plan
- Update Rust Codex app-server tests to remove the empty-answer expectation and assert that `requestUserInput` is not answered until the test submits answers.
- Add/adjust workspace integration tests matching the Python coverage:
  - pending request card is emitted while the turn is still active;
  - submitting by request id or question id wakes the live Codex request;
  - Codex receives the selected answer payload, then produces final assistant/plan output;
  - no live pending request expires the card instead of pretending delivery succeeded;
  - duplicate identical submissions are idempotent, conflicting duplicates fail.
- Run:
  - `cargo test -p spark-agent-adapter --test codex_app_server_contracts`
  - `cargo test -p spark-workspace --test conversation_event_normalization_contracts`
  - `cargo test -p spark-http --test workspace_conversation_turn_route_contracts`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`

## Assumptions
- Match Python’s user-visible behavior: wait indefinitely for a user answer unless the turn/app-server process ends or the pending request is no longer live.
- Keep the frontend request card API unchanged.
- Keep `turn/steer` for ordinary intervention/steering, not for answering an active `requestUserInput` JSON-RPC request.
