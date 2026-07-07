# First-Principles Conversation Record Model

## Summary
Replace conversation persistence with a small set of explicit record files and one typed commit boundary. Chat conversations and run/LLM-node transcripts share the same durable transcript record contract; runtime continuity and debug traces are separate authorities. One directory restores the UI, one journal replays committed changes, one runtime record resumes the model when possible, and optional traces debug upstream protocol behavior.

An earlier draft of this request (another model's take on the same prompt) proposed a similar target; its diagnosis was verified against the code and its design was re-derived from first principles before execution. Three decisions were confirmed with the user explicitly:
- **Keep a committed mutation journal** per conversation (audit trail and incremental reconnect catch-up; snapshot refetch remains the gap fallback). Snapshot-only recovery was considered and would be simpler, but the journal was chosen deliberately.
- **One-time migration** of legacy `state.json` conversations rather than a permanent dual read path or a hard drop.
- **Full run/chat transcript unification now**, not deferred.

## Diagnosis (verified)
The previous implementation made `state.json`, `events.jsonl`, live progress payloads, revision allocation, stale snapshot rebasing, sidecar merging, and transcript materialization compensate for one another:
- `state.json` was the durable render authority, but service paths mutated it as untyped JSON across a ~5,900-line service.
- `events.jsonl` was both a live replay cursor and a partial mirror of render mutations.
- Revisions were stamped twice: services pre-incremented via `touch_snapshot`, then the repository discarded and re-stamped them.
- The live event sink materialized stream deltas into an in-memory snapshot that completion handling discarded and reconciled against disk.
- Eleven artifact paths persisted full snapshots that bypassed the stale-rebase (gated to turn/segment batches) and could clobber a concurrently streaming turn; each also journaled and broadcast a full `conversation_snapshot` payload.
- Run/LLM-node transcripts duplicated the turn/segment model with separate types on backend and frontend, and the Runs UI rebuilt LLM progress rows from operational journal internals.

## Target Model
Per conversation, under `conversations/<id>/`:

1. `conversation.json` — stable metadata (schema version, id, handle, project path, settings, title, timestamps) plus the committed revision cursor. No transcript arrays, no protocol payloads.
2. `transcript.json` — canonical durable render state: ordered turns, ordered segments, inline artifact anchors by id and kind. Coalesced render values, never raw deltas. The same turn/segment contract serves project chat and run transcripts; run-only workflow boundary metadata rides outside the segment core.
3. `artifacts/<kind>.json` + `event-log.json` — artifact records (flow run requests, flow launches, run recoveries, proposed plans) and the workflow event log. Segments anchor artifacts by stable id; snapshot responses include artifacts by projection.
4. `journal.jsonl` — append-only committed mutations with repository-allocated, strictly increasing revisions. Replay cursors only; placement is always turn id plus segment id/order.
5. `runtime-session.json` — best-effort model continuity (provider, thread id, resume-failure tombstone). A separate failure domain from render state.
6. `codex-jsonrpc-trace.jsonl` — optional exact protocol transcript, debug only. Normal rendering, prompt construction, and replay never parse it or the runtime session.

Runs reuse the same typed transcript records (empty turn list, synthetic `run-node-*` turn ids, boundary segments) in `runs/<project>/<run>/transcript.json` with single-writer whole-file persistence — no run journal or commit boundary.

## Commit Boundary
`ConversationRepository::commit_conversation(conversation_id, project_path, base_revision, mutations) -> ConversationCommit` in `spark-storage` is the only write path.

Mutations are identity-keyed: `MetadataUpdated { patch }`, `TurnUpserted { turn }`, `SegmentUpserted { segment }`, `ArtifactUpserted { collection, artifact }`, `WorkflowEventAppended { event }`.

Repository responsibilities:
- load the latest committed record (running the one-time legacy migration if needed)
- apply mutations onto the latest state — a stale `base_revision` rebases by mutation identity instead of clobbering; rejection is reserved for unknown conversations with non-zero bases, segments targeting unknown turns, and unsupported stored schemas
- allocate segment orders and every journal revision
- maintain metadata: timestamps, title derivation, handle allocation
- write only the record files the batch touched, `conversation.json` last so an observed revision implies durable state, then append the stamped journal entries

Service responsibilities: construct typed mutations; never increment revisions; never hand-build journal envelopes; never persist raw provider events.

## Live Streaming Rule
Committed events carry a `revision`, are appended to `journal.jsonl`, and are replayable after reconnect. Transient `stream_delta` events carry a per-turn `stream_sequence` and the committed `base_revision` they render on top of; they are never journaled and may be dropped on reconnect. The frontend applies transients to the active view without advancing the committed revision; missed committed revisions recover through a fresh snapshot. During a streaming turn the only mid-turn commit is `request_user_input` (pending input must survive restart); everything else commits at turn completion.

## Migration Policy
One-time, on first read of a legacy conversation: validate the stored schema, project the merged legacy content (state.json plus project-level sidecar files) into the record files, seed `journal.jsonl` with a `conversation_snapshot` checkpoint at the carried-over revision (pre-migration replay cursors then recover naturally and revisions never regress), rename `state.json`/`events.jsonl` aside as `*.migrated`, and absorb the project-level sidecar files. Unsupported schemas error and leave every file untouched. No reconstruction from traces, ever.

## Non-Goals
- Durable transcript history as the primary model continuity mechanism.
- Replaying prior workspace transcript text into every prompt as a replacement for runtime sessions.
- Provider deltas or raw protocol notifications in the committed journal.
- Artifact records subordinate to transcript render state.
- Another run-only or LLM-node-only transcript shape.
