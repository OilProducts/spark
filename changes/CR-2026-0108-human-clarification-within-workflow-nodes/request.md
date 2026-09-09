# Human clarification within workflow nodes

## Summary

Allow Codex-backed agent nodes to call `request_user_input`, wait for an answer in Spark’s run UI, and continue the same execution. Questions also appear in the attention menu.

Reuse the existing Codex question protocol, durable run questions, and answer endpoint. No new graph node, outcome, or flow configuration is needed.

## Implementation

- Enable Codex’s `default_mode_request_user_input` feature for workflow agent sessions. Keep implementation nodes in their normal execution mode.
- Connect agent question requests to the runtime’s existing human-input mechanism. The runtime owns persistence and run status; the adapter owns the Codex tool response. Use a small callback between them rather than giving the adapter access to run storage.
- Persist each question with its owning run, node execution, agent request, and question ID. Support multiple questions in one request; deliver the tool result once all have answers.
- Reuse the existing questions and answer API, adding the metadata needed to distinguish agent clarification from graph gates. Validate answers against the active request and prevent duplicate submissions from replacing an accepted answer.
- Show questions in the existing run panel and attention menu, including questions from child runs. Support suggested choices and a custom text answer. Preserve question and answer history after submission.
- Mark the owning run as waiting while answers are outstanding, then running when the agent continues. Human waiting must not consume retries or trigger the adapter’s inactivity timeout. Keep cancellation, pause, and subprocess-exit detection responsive; unregister pending requests when execution ends.
- Add shared agent guidance to ask about unresolved user intent after inspecting available evidence. Update the evaluator prompt with the agreed instruction:

  > Evaluate whether the implementation achieves the user’s intended outcome, not merely whether it follows the plan. Verify assumptions using available evidence. Report concrete defects as implementation feedback; surface unresolved questions about user intent for clarification.

## Recovery boundary

Questions and accepted answers survive UI reloads and disconnects. If the executor dies, use Spark’s existing recovery behavior; this feature does not restore interrupted agent sessions.

Questions belonging to a dead or superseded execution remain in history but stop accepting answers. A retried execution receives distinct question identities, preventing an old answer from reaching a new request.

## Verification

- A fake Codex session requests input during normal execution, receives the submitted answer, and completes the same node without entering a retry loop.
- Multiple questions, custom answers, repeated requests, and child-run questions reach the correct tool call.
- Reloading the UI restores pending questions and accepted answers; duplicate or stale submissions cannot overwrite answers.
- Waiting beyond the inactivity threshold does not fail the node. Cancel, pause, and agent exit terminate the wait cleanly.
- Executor recovery preserves history without presenting an orphaned question as answerable.
- Existing graph gates and conversation questions continue to work.

## Defaults

Codex agent nodes only for this iteration; text-only nodes and other backends are unchanged. Use the run UI and attention menu, without mirroring questions into the originating conversation. No new dependency or automatic session-restoration system.
