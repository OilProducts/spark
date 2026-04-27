## Preserve Chat Continuity on Resume Failure

### Summary
Change project-chat session recovery so backend thread resume failures are explicit and non-lossy.

The new behavior should be:
- Spark must not silently start a fresh durable thread when a persisted thread cannot be resumed.
- Spark must preserve and surface the actual resume failure details.
- The attempted user turn should fail visibly, with a continuity-reset error recorded in durable conversation state and raw RPC/debug logs.
- A fresh thread may only be created later from an explicit new user action, not as an implicit fallback during the failed turn.

Also keep the prompt-template contract as-is: `recent_conversation` remains unsupported. The fix is to make failed thread reuse observable and safe, not to reintroduce transcript replay into prompts.

### Implementation Changes
- Update `CodexAppServerClient.resume_thread()` in `src/spark_common/codex_app_client.py` to return structured failure information instead of collapsing all resume errors to `None`.
  - Preserve any app-server error message/code.
  - Distinguish “resume failed” from “no thread id returned”.
- Update `CodexAppServerChatSession._ensure_thread()` in `src/spark/chat/session.py`.
  - If there is no persisted thread id, starting a new durable thread is still valid.
  - If there is a persisted thread id and resume fails, raise a dedicated runtime error instead of falling through to `start_thread()`.
  - Include the persisted thread id and the captured resume failure details in the raised error.
- Update `ProjectChatService` turn execution/failure handling in `src/spark/chat/service.py`.
  - Persist the failed assistant turn with a continuity-specific error message.
  - Keep the just-submitted user turn in the conversation history.
  - Do not mutate `session.json` to a new thread id on resume failure.
  - Append a clear raw-log/debug record that resume failed and no replacement thread was started.
- Add a small explicit error taxonomy for this path.
  - Introduce a named exception or error code for “persisted thread could not be resumed”.
  - Use wording that explains this is a continuity reset, not a generic runtime crash.
- Preserve the current prompt-template boundary in `src/spark/chat/prompt_templates.py`.
  - Keep `recent_conversation` unsupported.
  - Tighten the error text to make the rationale clearer: Spark does not replay prior transcript text into prompts; continuity depends on backend thread reuse, and failed reuse is now surfaced explicitly.
- Consider a small API/UI-facing signal in the conversation snapshot payload if already easy to thread through.
  - Minimum acceptable version: failed assistant turn error text only.
  - Better version: a stable error code the frontend can render as a continuity-reset card/banner.

### Test Plan
- Update existing resume tests in `tests/api/test_project_chat.py`.
  - Replace the current expectation that a new durable thread is started when resume fails.
  - Assert that resume is attempted once and `start_thread()` is not called.
  - Assert that the turn fails with continuity-reset messaging and preserves resume error details.
- Add a client-level test in `tests/spark_common/test_codex_app_client.py`.
  - Verify `resume_thread()` preserves app-server error message/code instead of returning bare `None`.
- Add a service-level turn test.
  - Seed a conversation with prior turns and a persisted thread id.
  - Simulate resume failure.
  - Assert the resulting conversation still contains the prior history and the new user turn, but the assistant turn is failed.
  - Assert no new thread id is written to `session.json`.
- Add raw-log expectations where practical.
  - Assert the outgoing `thread/resume` request and the captured resume failure are present.
  - Assert no fallback `thread/start` is issued for that turn.
- Keep the prompt-template contract tests.
  - `{{recent_conversation}}` remains rejected.
  - Custom prompts still only receive supported runtime variables.
  - Older message text is still not injected into prompt rendering by template expansion.

### Public/API Effects
- No change to the supported prompt variables.
  - `recent_conversation` stays unsupported.
  - `latest_user_message` remains the only conversation-content prompt variable.
- Runtime behavior change:
  - “resume failed” becomes a visible user-facing conversation error instead of a silent backend reset.
- If a stable error code is added to snapshots/segments, document it and keep it narrow to continuity-reset failures.

### Assumptions
- Chosen policy: fail the attempted turn on resume failure; do not automatically create a replacement durable thread.
- A later explicit retry or new message may create a fresh thread only after the failure is visible to the user.
- We are not reintroducing transcript replay into prompts as part of this fix.
- The likely trigger for the observed failure was the Codex runtime migration from temp runtime storage to `~/.spark/runtime/codex`, but the implementation should handle any future resume failure the same way.
