# CR-2026-0176: Coordinator handoff notes

Written 2026-11-02 for continuing this work on another machine. `request.md` is the brief the design was written from; `design.md` is the reviewed design (design-change run `run-18e45688e6dafe3e`). Neither has been implemented. This branch exists only to carry them; it is not meant to be merged as is.

## Does the design match the request?

Mostly. It finds the shared cause the request asked about: `conversation.json` publishes a revision number but no position in the logs, so every reader replays both logs from the start, and the commit cannot tell committed records from abandoned ones. Its fix is a published log cursor (schema 6): committed transcript length, last transcript revision, last journal event sequence, and versioned artifact files. Reads are cached per process and advanced by the bytes committed since the last read.

Covered:

- Server work before a send is accepted: revision-only reads, a cached snapshot, and a commit without full reads.
- Live replay: a backward scan of only the new tail of `events.jsonl`.
- Warm listing: no `events.jsonl` reads and a segment-free summary.
- All five integrity gaps: abandoned-commit revival, mixed-revision reads, unsynced logs before publish, unsynced renames, and pruning of a referenced attachment.

Gaps against the request and things to decide:

1. **"Sending a message feels immediate" is not fully covered.** The design defers showing the message on Enter (optimistic echo), and adds it only if send acceptance on the Coordinator conversation is still above ~100 ms after this change. That is a reasonable order, but the operator asked for immediacy. Either measure right after implementing, or do the small composer change regardless. The composer already records `setPendingConversationTurn({ conversationId, afterRevision })`; adding the message text there and rendering it until the server's turn arrives is a small, separate change.
2. **Every existing conversation becomes unreadable.** Schema 5 to 6 has no migration (AGENTS.md, Stored data). That includes the standing Coordinator and Chief of Staff conversations and their history. Listing skips them; the operator deletes `~/.spark/workspace/projects/*/conversations/*` after upgrading. This is allowed by policy, but it is the biggest user-visible consequence and should be a conscious choice.
3. **Cold start still replays every transcript once.** The first listing after a restart replays each `transcript.jsonl` (31.8 MB here) to fill the cache. Persisted checkpoints are deferred. If "after a restart" slowness matters, as it did for the earlier conversation-open stall, this is the next step.
4. **The final revisions were not independently re-reviewed.** The flow's review visit limit was reached after round 2.
5. **Unbounded in-process cache.** It holds one hydrated record for every conversation read since start-up; LRU eviction is deferred.

## Related state on main (11a0d8a4)

- Conversation-open stall fixed (a54ceac2); the running indicator comes from a server `running` flag, with no full-snapshot polling (3903f3de).
- Activity JSONL reads tolerate a torn final record after power loss, and runtime failures retry once and then stop the run (9c6e9487).
- The investigation of the send delay (`run-18e455c08a737769`) measured browser-side snapshot handling at ~15-20 ms; the delay is server accept time. Per-phase POST timings were not measured.
- Other open audit items (`run-18e4558660388217`), outside this design: run live publication rebuilds token usage from all executions on every publish (`crates/spark-http/src/lib.rs:690-727`); a slow poll trigger blocks scheduled triggers, and a new HTTP client is built every tick.
