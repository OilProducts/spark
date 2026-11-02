# CR-2026-0176: Chat performance and integrity, looked at together

## Request (operator, 2026-11-02)

"Lets see if we can improve *all* of the chat related items all at once... Launch the design change flow and see if theres something fundamental."

Earlier, about this standing Coordinator chat: "when I type a message to you here and press enter there is a noticable delay before it is rendered."

## What is known

Scale on this machine: ~40 conversations; the standing Coordinator conversation (`role-spark-8d0808e69862-coordinator`) has a ~4.5 MB `transcript.jsonl`, a 13.7 MB `events.jsonl`, 270 turns and 777 segments; its snapshot is ~2.7 MB and a GET takes ~90 ms on the server.

From investigate-bug run run-18e455c08a737769 (send delay):
- The composer shows nothing until the server accepts the turn (`frontend/src/features/projects/hooks/useConversationComposer.ts:113-190`); the message appears from the live stream (`useConversationStream.ts:180-213`) or the send response.
- Browser-side cost of a full snapshot was measured as small: ~10 ms body, ~3-6 ms JSON parse, ~1 ms validation, ~1-4 ms hydration (8 samples, headless Chromium).
- One send rebuilds the whole conversation from disk several times before it is accepted: `current_conversation_revision` (`crates/spark-http/src/workspace.rs:431`), `begin_turn` (`crates/spark-workspace/src/conversations.rs:1228`) and its full-history build (`:1504`), the commit (`crates/spark-storage/src/conversation/commit.rs:42-72`), live publication (`crates/spark-workspace/src/live.rs:468`) and the summary (`live.rs:1011` -> `conversations.rs:626`). Each read goes through `conversation/store.rs:106-157` (activity events + transcript hydration + artifacts). Per-phase POST timings were not measured.

From the performance/integrity audit run-18e4558660388217 (source inspection, not timed):
- Incremental conversation event reads parse the whole activity log before filtering (`crates/spark-storage/src/workspace_conversations.rs:704-729`, `activity.rs:187-189`); turn progress and live replay use this path.
- Listing conversations reads every snapshot in full to build summaries (`conversations.rs:660-704,6361-6435`).
- A commit that appends event and transcript records but stops before publishing `conversation.json` can have its abandoned record revived by the next commit (`commit.rs:57-76,173-235`, `store.rs`).
- Readers can combine an old `conversation.json` revision with newer artifact files, because `read_record` reads artifacts without revision filtering or a reader lock (`store.rs:156-168,192-224`).
- Attachment cleanup can delete an attachment directory a committed turn still references when its `attachment.json` is unreadable (`conversation/attachments.rs:178-200,236-243`).
- Atomic writes do not sync the parent directory after rename (`crates/spark-storage/src/atomic.rs:10-35`).

Recently changed in this area (on main): conversation summaries carry a server `running` flag pushed over the live stream (CR-2026-0175, 3903f3de); activity JSONL reads tolerate a torn final record after power loss (9c6e9487).

## Wanted

Find whether these share a fundamental cause in how conversations are stored, read and published, and design the change that fixes them together: sending a message feels immediate, and send, live updates, listing and opening scale with what changed rather than with total history, without the integrity gaps above.

Project policy (AGENTS.md, Stored data): older on-disk formats are not supported; no migrations or compatibility code, old history may be wiped.
