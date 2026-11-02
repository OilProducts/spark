# CR-2026-0176 design: publish a log cursor, read conversations incrementally

## Problem

Sending a chat message, live updates, listing and opening a conversation all
cost time in proportion to the conversation's whole history. Several
integrity gaps sit on the same code. They share one cause:

**`conversation.json` publishes only a revision number, and that number
cannot locate committed data in the logs.** So every reader rebuilds the
conversation from the start of both logs, and the commit cannot tell
committed records from abandoned ones.

How this plays out in the current code:

1. **Every read replays both logs in full.** `read_record`
   (`crates/spark-storage/src/conversation/store.rs:106-188`) parses the whole
   of `events.jsonl` (`activity.read_events()`, `:141-153`). It does this only
   to find the highest event sequence whose `revision <= meta.revision`. It
   then replays the whole of `transcript.jsonl` up to that sequence
   (`hydrate_transcript_through_event_sequence`,
   `crates/spark-storage/src/activity.rs:211-233`). Nothing is cached:
   `ConversationRepository` is built fresh on each call
   (`crates/spark-workspace/src/conversations.rs:4096-4097`). Its `activities`
   map (`crates/spark-storage/src/workspace_conversations.rs:396-401`) is
   therefore per-call, and only `append_provider_event` uses it. On this
   machine the Coordinator conversation has an 18.8 MB `events.jsonl` and a
   6.1 MB `transcript.jsonl`. Across all 45 conversations there is 87.5 MB of
   events and 31.8 MB of transcript.
2. **One send does that replay many times.**
   - The HTTP handler reads the revision through a full snapshot
     (`crates/spark-http/src/workspace.rs:431`, `:1543-1553`).
   - `begin_turn` reads the snapshot (`conversations.rs:1265`).
   - The commit reads it again (`commit.rs:59`). It also parses all of
     `transcript.jsonl` just to count records (`commit.rs:181`).
   - Live publication calls `get_snapshot` only to learn the revision
     (`crates/spark-workspace/src/live.rs:475`). It then parses all of
     `events.jsonl` again to replay events after that revision
     (`live.rs:487` → `workspace_conversations.rs:704-727`).
   - The summary push reads the snapshot again (`live.rs:1011-1021` →
     `conversations.rs:626-643`).
   - `execute_turn_with_progress_payloads` and the queued-turn loop
     read full snapshots just for a revision (`conversations.rs:1949-1953`,
     `:1981-1987`). They then call the same full-log `read_events_after`
     (`:1956`, `:1994`).
3. **Listing replays every conversation.** `list_project_conversations`
   (`conversations.rs:660-704`) and the attention list
   (`conversations.rs:571-613`) call `read_snapshot` on every conversation.
4. **An abandoned commit is revived.** The commit appends journal events
   stamped with revisions `N+1..N+k` and their transcript records. It writes
   `conversation.json` last (`commit.rs:183-230`). If it stops before that
   write, the next commit reads revision `N` again and reuses revisions
   `N+1..`. Once that commit publishes, `read_record`'s filter
   `revision <= meta.revision` (`store.rs:141-153`) matches the abandoned
   events too. Their transcript records are then hydrated as committed. The
   same duplicate revisions also corrupt `read_conversation_events_after`
   (`workspace_conversations.rs:715-726`), which collects every event with
   `revision > R` and sorts by revision.
5. **A reader can mix revisions.** `read_record` reads `conversation.json`,
   then the artifact files, with no lock (`store.rs:107-164`). A commit
   writes the artifact files before `conversation.json` (`store.rs:192-218`).
   A concurrent reader can therefore pair an old revision with new artifacts.
6. **The committed transcript is not durable before it is published.**
   `append_activity_record` (`activity.rs:330-346`) never calls fsync. So
   after a power loss, `conversation.json` (which is fsynced through
   `write_atomic`) can claim records that were lost. This breaks the
   invariant stated at `store.rs:6-7` and `commit.rs:227-229`.
7. **Renames are not durable.** `write_atomic`
   (`crates/spark-storage/src/atomic.rs:10-36`) never fsyncs the parent
   directory after `persist`, so the rename itself can be lost.
8. **Pruning can delete a referenced attachment.** `prune_attachments`
   (`crates/spark-storage/src/conversation/attachments.rs:181-200`) checks
   the reference set only when `attachment.json` parses (`:183`). An
   unreadable record falls through to the directory mtime (`:185-188`), so a
   directory that a committed turn still references gets deleted.

The browser side of a send is small: about 15-20 ms in total for body, parse,
validation and hydration (investigate-bug run run-18e455c08a737769). The
delay before the message renders is the server's accept time, items 1-2.

## Goals and non-goals

Goals:

- A warm read (snapshot, revision, summary) does no log parsing beyond the
  bytes committed since the last read in this process. A revision-only read
  touches only `conversation.json`.
- Send acceptance does no full-history disk reads. Its remaining
  history-sized work is in memory: one record clone and projection, and the
  existing agent-history build.
- Live replay after revision `R` reads only the tail of `events.jsonl` written
  since `R`.
- Listing reads one small file per conversation and, for each conversation
  already cached in this process, applies only the transcript bytes committed
  since its last read. It never opens `events.jsonl`. The first listing after
  a restart replays each `transcript.jsonl` once to fill the cache (see the
  cold-start non-goal below).
- Close integrity gaps 4-8 above.

Non-goals (skipped, with when to add them):

- **Optimistic echo of the sent message in the composer**
  (`useConversationComposer.ts:165-194`). Add it if accept latency on the
  Coordinator conversation, measured after this change, is still above about
  100 ms.
- **A persisted transcript checkpoint for cold starts.** The first read of
  each conversation after a restart still replays `transcript.jsonl` (but no
  longer `events.jsonl`). Add a checkpoint when cold listing after a restart
  is too slow.
- **A bound on the in-process record cache.** Add LRU eviction when resident
  memory matters: 31.8 MB of transcript on disk here hydrates to less.
- **Shrinking full-snapshot live payloads** for `conversation_snapshot_ref`
  commits (`live.rs:527-533`, `journal.rs:89-105`). Browser cost was measured
  as small.
- **Speeding up `abandon_active_turn`'s full `events.jsonl` scan**
  (`conversations.rs:2243-2246`). It is a recovery path, not a hot one.
- **Node-execution activity** (attractor-runtime). It uses
  `ActivityRepository` but not `read_record`, and stays as it is.

## Approach

### 1. `conversation.json` publishes a log cursor (schema 6)

`conversation.json` gains a `log` object next to the existing metadata keys:

```json
"log": {
  "transcript_bytes": 6137215,
  "transcript_revision": 8123,
  "event_sequence": 40211,
  "artifacts": { "event_log": 977, "flow_launches": 1012, "flow_run_requests": 3, "run_recoveries": 640, "proposed_plans": 3 }
}
```

- `transcript_bytes` is the length of the committed prefix of
  `transcript.jsonl`. Readers never look past it.
- `transcript_revision` is the dense revision of the last committed
  transcript record. The commit uses it in place of
  `read_transcript_records().len()` (`commit.rs:181`).
- `event_sequence` is the sequence of the commit's last journal event in
  `events.jsonl`. It is the anchor for live replay (step 4).
- `artifacts` names the version of each artifact file that belongs to this
  revision. Each artifact collection and the workflow event log is stored
  as `artifacts/<name>.<version>.json`, where `<version>` is the final
  revision of the commit that last wrote it. Readers open exactly the named
  version, never "whatever file is there". A collection that has never been
  written is absent from the map and reads as empty. This replaces the
  fixed names `artifacts/<name>.json` and `event-log.json`
  (`store.rs:23-24`, `:44-52`, `:65-72`).

The cursor lives in a private storage struct that flattens `ConversationMeta`
plus `log`. It is not part of `ConversationMeta`, `ConversationRecord` or the
snapshot JSON, so no API payload changes.

`CONVERSATION_STATE_SCHEMA_VERSION` goes from 5 to 6
(`workspace_conversations.rs:18`). `read_record` checks `schema_version`
before it requires `log`. A schema-5 file therefore fails with the existing
`InvalidConversationState` / `UNSUPPORTED_CONVERSATION_STATE_SCHEMA` error,
which listing already skips (`conversations.rs:701`). It does not fail with a
parse error that would break the whole list. Following AGENTS.md (Stored
data), there is no migration: existing conversations must be deleted.

### 2. Commit: truncate the abandoned tail, sync, then publish

In `commit_conversation` (`commit.rs:42-240`), under the existing exclusive
`.commit.lock`:

1. Load the latest record through the cache (step 3), using the lock-free
   inner read: the commit already holds the lock. This replaces
   `read_snapshot` + `record_from_snapshot` (`commit.rs:59-69`).
2. If `transcript.jsonl` is longer than `log.transcript_bytes`, truncate it
   to that length. This drops the transcript records of an abandoned batch
   (gap 4). Only the conversation commit appends to a conversation's
   `transcript.jsonl`, always under this lock, so the truncation cannot race
   another writer. `events.jsonl` is **not** truncated:
   `append_provider_event` (`workspace_conversations.rs:664-702`) appends
   provider events to it without the commit lock, and those events are
   needed for recovery. Abandoned journal events stay in the file, and
   step 4 ignores them.
3. Append events and transcript records as today. The first transcript
   revision is `log.transcript_revision + 1`.
4. `sync_data` both `transcript.jsonl` and `events.jsonl` once, after the
   appends and before any artifact or `conversation.json` write (gap 6).
   Syncing the events file matters for live replay, not only recovery:
   event sequences are allocated from the surviving file tail
   (`activity.rs:130-138`). Without the sync, a power loss after publication
   could drop a published journal event. A surviving abandoned event with
   the same revision would then be replayed in its place (see the Review
   section). With the sync, every event at or below a published
   `event_sequence` is durable.
5. Write each touched artifact collection to a **new** file
   `artifacts/<name>.<final_revision>.json` through `write_atomic`. Then
   write `conversation.json` with the new `log`:
   - `transcript_bytes`: the file length after the appends.
   - `transcript_revision`: the last revision appended.
   - `event_sequence`: the sequence of the last journal event appended.
   - `artifacts`: the previous map, with each touched name set to
     `final_revision`.

   If the commit appended no journal event, keep the previous
   `event_sequence`. An interrupted commit leaves only unreferenced version
   files behind. No reader opens them, and the published versions are never
   overwritten.
6. After `conversation.json` is written, delete best-effort every file in
   `artifacts/` that the new map does not name. This removes superseded
   versions and an earlier abandoned commit's leftovers. A failed delete
   only leaves a stray file, which the next commit removes.

### 3. Reads: shared lock, cursor-validated in-process cache

`read_record` (`store.rs:106-188`) is the single path behind `read_snapshot`,
`read_existing_snapshot` and the commit. It becomes:

1. Take a **shared** `fs2` lock on the conversation's `.commit.lock`
   (gap 5). Commits hold it exclusively through the `conversation.json`
   write. Read `conversation.json` while holding the lock. If the file is
   missing, return `None` and drop any cache entry.
2. Look up a process-wide cache keyed by conversation root path: a `static`
   `OnceLock<Mutex<HashMap<PathBuf, …>>>` in `store.rs`. Each entry holds
   the cursor it was built at, the transcript **in replay order** (upserts
   applied, not yet sorted), and the artifacts. The global map lock is not
   held across file I/O.
   - Same `transcript_bytes` and revision: a hit, with no file reads beyond
     `conversation.json`.
   - Cached `transcript_bytes` ≤ new: read only the byte range
     `[cached, new)` of `transcript.jsonl` and apply those records with the
     existing `upsert_turn` / `upsert_segment` / `remove_segment` rules.
   - Revision changed: re-read only the artifact files whose version in
     `log.artifacts` differs from the cached one. All five together are
     about 150 KB for the Coordinator. Because a version file is never
     rewritten once named, a cold reader and a warm reader at the same
     cursor see identical artifacts.
   - Cached bytes > new, or no entry: replay `[0, new)` in full.
   - `transcript.jsonl` shorter than `transcript_bytes`: return
     `InvalidConversationState`. A published cursor past the durable log
     means corruption.
3. The returned record is a clone of the entry with the existing stable
   `segments.sort_by_key(order)` applied (`activity.rs:231`), so the result
   is identical to a full replay. It also carries the existing plan-link
   derivation (`store.rs:165-182`). Steps 1-2 (lock, cursor check,
   incremental apply) are one internal function that ends with the entry
   up to date. Two outputs are built from it while the map lock is held:
   the full record (this step), and the summary record (section 5), which
   never clones or sorts segments.
4. `delete_conversation` (`workspace_conversations.rs:729`) drops the entry.

Readers no longer open `events.jsonl`. Another process's commits are picked
up because every read re-reads `conversation.json`, the publication point.
The commit does not write into the cache. The next read applies the bytes
that commit appended, so there is one code path.

Add a range reader `ActivityRepository::read_transcript_records_range(from, to)`
next to `read_transcript_records_from` (`activity.rs:198-203`). It is the
same strict decoder as `read_activity_records` but stops at `to`. Inside the
cursor, every record is newline-terminated and valid, so an invalid or
unterminated record there is an error.

The commit's inner read and `referenced_attachment_ids`
(`attachments.rs:208-222`) already run under the exclusive lock. They call a
lock-free variant: taking the shared lock on a second fd would deadlock
against the caller's own exclusive `flock`.

### 4. Live replay scans `events.jsonl` backwards to the requested revision

`read_conversation_events_after(R)` (`workspace_conversations.rs:704-727`)
works as follows:

1. Read `conversation.json` under the shared lock, giving revision `P` and
   `log.event_sequence` `S`. If `R >= P`, return an empty list.
2. Scan `events.jsonl` backwards from end of file in 64 KiB chunks, parsing
   each line once. Skip events with sequence `> S` (unpublished) and events
   without `revision` (provider events). The **first** journal event
   accepted must be at sequence exactly `S` with revision `P`; if the event
   at `S` is missing or has another revision, return an empty list. After
   that, accept a journal event only when its revision equals `expected`,
   which starts at `P - 1` and decrements on each acceptance. Stop when
   `expected == R`.
3. Return the accepted events in ascending order. If the scan reaches the
   start of the file first, return an empty list. Callers already treat that
   as `resync_required` (`live.rs:488-495`), and the progress callers simply
   publish nothing (`conversations.rs:1955-1959`, `:1993-1998`).

Why this never returns an abandoned event: an abandoned batch stamps
revisions above the revision published at the time, and the commit that
replaces it reuses those revisions **later in the file**, at higher
sequences. Scanning backwards from `S`, the published event for each
revision is therefore met before any abandoned event with the same
revision. After acceptance moves past that revision, the abandoned one no
longer equals `expected`. The anchor check at `S` plus the events sync
(step 2.4) guarantee that the published event is actually present, so the
scan cannot fall through to an abandoned copy. The cost is proportional to
the bytes written since `R`.

**One backward scanner.** `tail_record` (`activity.rs:412-438`) and
`read_event_suffix` (`activity.rs:440-473`) already each carry their own
chunked backward loop. Replace both loops with one private helper,
`scan_backwards<T>(path, visit: impl FnMut(T) -> ControlFlow<()>)`. It reads
64 KiB chunks from end of file, handles line boundaries with the existing
`activity_lines` rules, and passes records newest first until `visit` breaks.
`tail_record`, `read_event_suffix` (used by `uncommitted_event_suffix`,
`activity.rs:236-241`) and the new events-after scan all become short
callers of it. After this step, one implementation of backward chunk reading
remains.

### 5. Callers that only need a revision or a summary

- `ConversationRepository::read_revision(id, project) -> Result<Option<i64>>`
  reads `conversation.json` only. Use it in:
  - `current_conversation_revision` (`spark-http/src/workspace.rs:1543-1553`,
    through a one-line service wrapper)
  - `conversations.rs:1949-1953` and `:1981-1987`
  - `conversation_envelopes` (`live.rs:475-476`). It builds the snapshot only
    when it will send one: no revision given, a `conversation_snapshot_ref`
    in the replay, or `resync_required`.
- `ConversationRepository::read_summary_snapshot(id, project)` shares
  steps 1-2 of the cached read (section 3): a `conversation.json` read, plus
  the transcript bytes committed since this process last read the
  conversation, or a full transcript replay on the first read after a
  restart. It then builds its output straight from the cache entry, not from
  a full-record clone: a `ConversationRecord` whose `meta`, `turns` and
  artifact collections are cloned from the entry, and whose `segments` and
  `event_log` are left empty. No segment is cloned, sorted or serialized.
  The plan-link step is skipped, since it only touches segments.
  `snapshot_from_record` (`projection.rs:46-108`) then projects that record
  unchanged, emitting `segments: []` and `event_log: []`. It still includes `turns` (needed for the
  preview, the title fallback and `running`) and `flow_launches` (needed for
  `launched_run_ids`). Segments are the bulk of the bytes, and neither
  omitted field is read by
  `conversation_summary_from_snapshot` (`conversations.rs:6361-6429`, which
  only checks that `segments` exists at `:6371`), `active_assistant_turn_id`
  (`:4395`) or the attention list. Use it in `list_project_conversations`
  (`:680`), `thread_list_summary` (`:638`, replacing `get_snapshot`) and the
  attention list (`:575`).

### 6. Smaller integrity fixes

- `write_atomic` (`atomic.rs:28-34`): after `persist`, open the parent
  directory and `sync_all` it, under `#[cfg(unix)]` (gap 7). Windows builds
  exist (`spark-agent-adapter/src/local_environment.rs:673`), and there a
  directory cannot be opened this way.
- `prune_attachments` (`attachments.rs:181-189`): skip any entry whose
  directory name is in `referenced`, before `attachment.json` is read
  (gap 8). Attachment directories are named by id (`attachments.rs:59-61`).

### What one send costs afterwards

| Step | Before | After |
|---|---|---|
| Revision read in the HTTP handler | Full replay of both logs | One read of `conversation.json` |
| Snapshot read in `begin_turn` | Full replay | Cache hit plus an in-memory clone and projection |
| Agent history build | In memory | In memory (unchanged) |
| Commit | Full replay plus a full transcript count | Cache hit, a few appends, one fsync, the existing atomic writes |
| Live publication | Full `events.jsonl` parse | Backward scan of the bytes written since the request began |
| Summary push | Full replay | Cache hit plus the segment-free projection |

## Alternatives considered

- **Rewrite a materialized `state.json` on every commit.** This makes reads a
  single parse, but every commit then rewrites the whole snapshot: 2.7 MB per
  segment upsert during streaming. The cost moves into the commit that
  blocks sending, and the log-revival and journal-replay problems stay. It
  lost on commit cost.
- **SQLite per conversation or per project.** Transactions would give
  atomicity, reader isolation and indexed reads in one step. But it is a new
  dependency, a full storage rewrite of `spark-storage::conversation`, and
  every test fixture that seeds conversation files changes. It is far larger
  than the request needs, because the existing append-only logs plus a
  published cursor already provide each of those properties.
- **Only an in-memory cache keyed by file size or mtime, with no on-disk
  cursor.** This is smaller, but readers would still need `events.jsonl` to
  find the published prefix. It cannot tell abandoned records from committed
  ones (gap 4) and does not fix live replay. It lost because it leaves the
  root cause in place.

## Risks

- **Existing conversations become unreadable** at schema 6, including the
  standing Coordinator conversation's 270 turns. The policy in AGENTS.md
  allows this, and the request restates it. Listing skips them, and the
  standing role shows its placeholder row (`conversations.rs:705-725`). The
  operator deletes `~/.spark/workspace/projects/*/conversations/*` once after
  upgrading. Spark does not delete anything automatically.
- **Cache coherence.** Correctness depends on every writer of
  `transcript.jsonl` and artifact files going through the commit lock and
  publishing `conversation.json` last. That is true today for conversations,
  because the commit is the only writer. If another process deletes a
  conversation and recreates it with the same id, and the new log grows past
  the cached length before this process reads again, this process would
  serve a stale prefix. Deletes from this process drop the entry. The window
  only exists with two Spark processes on one home directory, and the
  implementation should note it with a `ponytail:` comment.
- **Incremental apply must equal a full replay.** Keeping the cached
  transcript in replay order and sorting on output reproduces
  `hydrate_transcript` exactly. A contract test compares the two directly.
- **Shared-lock readers wait for commits.** Commits now do no full reads,
  so the wait is only the appends and writes. Any code path that reads
  while holding the exclusive lock must use the lock-free variant, or it
  deadlocks. There are two such paths today: the commit and
  `referenced_attachment_ids`. A test covers each.
- **Memory.** The cache keeps one hydrated record for each conversation read
  since start-up. Mark the cache with a `ponytail:` comment naming LRU as
  the next step.
- **fsync cost.** Each commit adds two log syncs (`transcript.jsonl` and
  `events.jsonl`) and one directory sync per atomic write. These are milliseconds on SSD and far
  below the replay cost they replace.

## Implementation plan

### Files

| File | Action | Change |
|---|---|---|
| `crates/spark-storage/src/workspace_conversations.rs` | modify | Schema 6. `read_revision`, `read_summary_snapshot`. Backward-scan `read_conversation_events_after`. Cache eviction in `delete_conversation`. |
| `crates/spark-storage/src/conversation/store.rs` | modify | Stored cursor struct with the artifact version map, versioned artifact file paths, shared-lock cached `read_record` plus a lock-free inner variant, `write_record` writes new version files then `log`, then removes unnamed files. |
| `crates/spark-storage/src/conversation/commit.rs` | modify | Inner cached read, truncate the abandoned transcript tail, revision from the cursor, sync both logs before publish, write the cursor. |
| `crates/spark-storage/src/conversation/attachments.rs` | modify | Prune skips referenced directory names first; use the lock-free read. |
| `crates/spark-storage/src/activity.rs` | modify | `read_transcript_records_range`. One `scan_backwards` helper replacing the loops in `tail_record` and `read_event_suffix`, used by the new events-after scan. A helper that syncs both logs. |
| `crates/spark-storage/src/atomic.rs` | modify | Parent-directory fsync on unix. |
| `crates/spark-workspace/src/conversations.rs` | modify | `read_revision` at `:1949`, `:1981`. Summary snapshot in list, attention list and `thread_list_summary`. A service `read_revision` wrapper. |
| `crates/spark-workspace/src/live.rs` | modify | `conversation_envelopes` reads the revision first and builds the snapshot only when it sends one. |
| `crates/spark-http/src/workspace.rs` | modify | `current_conversation_revision` uses `read_revision`. |
| `crates/spark-http/tests/contracts.rs` | modify | The conversation-seeding fixture (`:26-100`, which hand-writes `conversation.json`, `event-log.json` and `artifacts/*.json`) writes `log` and versioned artifact files, or commits through the repository. |
| `crates/spark-workspace/tests/contracts.rs` | modify | The same fixture change. |
| `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs` | modify | New contract tests (below). |
| `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs` (same row as above) | modify | Also holds the events-after, summary and listing tests. `workspace_conversation_contracts.rs` is not registered in `crates/spark-storage/tests/contracts.rs` and expects legacy migration, so it is left untouched and no test is added there. |
| `crates/spark-storage/tests/contracts/activity_repository_contracts.rs` | modify | Range reader tests. |

### Steps (each leaves `just test` green)

1. **Durability helpers.** Add the parent-directory fsync in `write_atomic`,
   the prune fix, and `read_transcript_records_range`, each with its test.
   No format change.
2. **Cursor and schema 6.** Write `log` in `write_record` and require it in
   `read_record`, with the schema check first. The commit computes the
   cursor, truncates the transcript to `transcript_bytes`, syncs both logs
   before publishing, takes its transcript revision from the cursor, and
   writes artifacts as versioned files named in `log.artifacts`.
   `read_record` reads transcript bytes `[0, transcript_bytes)` and stops
   reading `events.jsonl`. Update the seeding fixtures and any test that
   hand-writes `conversation.json`:
   - `spark-http/tests/contracts.rs:82`
   - `spark-storage/tests/settings_documents.rs:194`
   - `conversation_commit_contracts.rs:159-181`

   The `profile_settings.rs:171` fixture only exercises profile-reference
   scanning, which does not parse `log`. Leave it unless it fails.
3. **Shared lock and cache** in `read_record`, with the lock-free inner
   variant used by the commit and `referenced_attachment_ids`, and eviction
   on delete.
4. **One backward scanner, then backward-scan
   `read_conversation_events_after`.** First move `tail_record` and
   `read_event_suffix` onto `scan_backwards`; the existing activity
   contract tests (including the crash-tail test at
   `activity_repository_contracts.rs:156`) must pass unchanged. Then add the
   events-after scan on top of it.
5. **Revision-only and summary reads** in workspace, live and http.
6. Run `just test` and record the timing measurement below.

### Test plan

New Rust tests go in the existing `tests/contracts/` modules, with no new
top-level test files.

- **The abandoned batch is not revived.** Commit `N`. Then, with
  `ActivityRepository`, append a journal event with revision `N+1` and a
  transcript `TurnUpsert` for turn `ghost`, without touching
  `conversation.json`. Commit a different turn. Assert:
  - the snapshot has no `ghost`;
  - `transcript.jsonl` holds no `ghost` record;
  - `read_conversation_events_after(N)` returns exactly the second commit's
    events, in order.
- **An abandoned journal event is not replayed in place of a published
  one.** Commit revision 1. Append, without publishing, a journal event with
  revision 2 labelled `ghost`. Commit a real revision 2 (it lands at a
  higher sequence). Assert `read_conversation_events_after(1)` returns the
  real event. Then write a `conversation.json` whose `event_sequence` points
  past the end of `events.jsonl` (simulating a lost published event) and
  assert the result is empty, not `ghost`.
- **An abandoned artifact write is not visible.** Commit an artifact `A`.
  Write `artifacts/flow-launches.<N+1>.json` holding `A'` without
  publishing. Assert that a cold read (new process cache: drop the entry)
  and a warm read both return `A`. Then commit another change and assert
  that the stray file is gone and the record still holds `A`.
- **Bytes past the cursor are ignored.** Append a valid transcript record
  after the cursor and assert that it is not read. Assert that a cursor
  longer than the file returns `InvalidConversationState`.
- **Incremental equals full replay.** Run a sequence of commits on one
  repository, reading after each one:
  - a queued turn moved to the end;
  - a segment upsert that changes `order`;
  - a tombstone;
  - an artifact change.

  After each read, compare `read_snapshot`'s `turns` and `segments` with
  `ActivityRepository::hydrate_transcript()` over the same root.
- **Cursor invalidation.** Warm the cache, then publish a commit through a
  second `ConversationRepository` instance and read through the first. Also
  delete and recreate the conversation, and assert that the old turns are
  gone.
- **Readers wait for an exclusive commit lock.** A test thread holds an
  `fs2` exclusive lock on `.commit.lock`. `read_snapshot` on another thread
  has not returned while the lock is held and returns after it is released.
- **No self-deadlock.** `prune_attachments` and `commit_conversation` both
  complete; existing tests already cover them.
- **Events after `R`.** The scan:
  - skips provider events;
  - returns an empty list for `R >= P`;
  - returns an empty list when `R` predates the file;
  - matches the old full-scan result on a multi-commit conversation.

  The existing `live.rs` resync contracts keep passing.
- **Prune keeps a referenced attachment** whose `attachment.json` is corrupt
  and whose directory mtime is older than the cutoff.
- **Summary snapshot matches the full one.** For a conversation with
  turns, segments, flow launches and a workflow event, `read_summary_snapshot`
  equals `read_snapshot` with `segments` and `event_log` set to `[]`, and
  `conversation_summary_from_snapshot` gives the same summary for both.
- **Schema 5 is rejected.** A schema-5 `conversation.json` without `log` is
  rejected with `InvalidConversationState` and is skipped by
  `list_project_conversations`, not turned into a list error.
- **Measurement (manual, recorded in the run result).** Seed a conversation
  of Coordinator size: about 270 turns and 800 segments, with a
  `transcript.jsonl` of 5 MB or more. Time `POST …/turns` acceptance and
  `GET` snapshot, warm, before and after the change. The target is a warm
  revision read under 1 ms and a send acceptance that no longer grows with
  log size.

## Open questions

None that change behaviour or scope. The only user-visible consequence, that
existing conversations must be deleted, follows the Stored data policy the
request cites.

## Review

Findings from the first design review, and how each was resolved. None was
contested.

1. **Blocker: an unsynced `events.jsonl` can replay an abandoned journal
   event after power loss.** Fixed. Step 2.4 now syncs `events.jsonl` as
   well as `transcript.jsonl` before publishing, so every event at or below
   a published `event_sequence` is durable. Step 4.2 adds an anchor check:
   the first accepted event must be at sequence exactly `S` with revision
   `P`, or the scan returns nothing and the caller resyncs. Section 4
   explains why scanning backwards from that anchor always meets the
   published event for a revision before any abandoned copy. A contract
   test covers both the normal case and a lost anchor.
2. **Major: the summary read path was underspecified and contradicted the
   listing goal.** Fixed. The listing goal now states the real cost: warm
   listing applies only newly committed transcript bytes and never opens
   `events.jsonl`; the first listing after a restart replays each
   transcript once, which is the existing cold-start non-goal. Section 5
   states that `read_summary_snapshot` uses the same cached `read_record`
   and that its saving is the segment-free projection. It lists which
   fields it keeps and why.
3. **Major: a second backward scanner beside the existing one.** Fixed.
   Section 4 and implementation step 4 replace the loops in `tail_record`
   and `read_event_suffix` with one `scan_backwards` helper, which the new
   scan also uses.
4. **Major: an interrupted artifact write could become visible.** Fixed.
   Artifact files are now versioned (`artifacts/<name>.<revision>.json`),
   and `conversation.json`'s `log.artifacts` names the version belonging to
   each published revision (sections 1, 2.5-2.6, 3). A commit never
   overwrites a named version, so cold and warm readers at one cursor see
   the same artifacts, and an abandoned version is never read and is
   removed by the next commit. A contract test covers it.

Findings from the second design review. None was contested.

5. **Major: the summary path still cloned and sorted every segment.**
   Fixed. Section 3.3 now splits the cached read into a shared update step
   and two outputs. Section 5 builds the summary record from the cache
   entry directly, with no segments or event log, so nothing is cloned or
   sorted beyond meta, turns and artifacts.
6. **Major: tests were planned for an unregistered module.** Fixed. The
   events-after, summary and listing tests go in
   `conversation_commit_contracts.rs`, which `tests/contracts.rs`
   registers. `workspace_conversation_contracts.rs` stays unregistered and
   unchanged. Whether to delete that stale module is outside this request.

