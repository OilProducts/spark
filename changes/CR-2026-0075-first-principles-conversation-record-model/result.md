# Result

Executed in seven phases, each independently green on the full validation gate (`cargo fmt --all -- --check`, `cargo test --workspace --all-features`, `npm --prefix frontend run test:unit`, `npm --prefix frontend run build`).

## Phase commits

1. `60195c8` — Add typed conversation records and commit boundary. `spark_storage::conversation`: `ConversationMeta`, `Transcript` turn/segment records with lossless extras, identity-keyed mutations, journal entries, transient event types, shared segment-id derivation, snapshot projections, and `commit_conversation` writing the legacy file shapes byte-compatibly beside the old path.
2. `ffa039f` — Route conversation persistence through the typed commit boundary. Every service write (turn start, completion ingest, backend failure, request-user-input answer/expiry, settings, all eleven artifact paths) builds mutation batches; `touch_snapshot` and the `persist_snapshot` wrappers deleted; identity-keyed artifact commits fix the concurrent clobber of streaming turns.
3. `1ba3ac8` — Split live conversation delivery into committed and transient events. Non-committed live sink emissions became `stream_delta` payloads (per-turn stream sequence + base revision) on `conversation.stream_delta` envelopes, never journaled or replayed; the frontend applies them without advancing the committed revision.
4. `c6c73c9` — Split conversation storage into record files with one-time migration. `conversation.json`, `transcript.json`, `artifacts/<kind>.json`, `event-log.json`, `journal.jsonl`; legacy conversations migrate on first read with a snapshot checkpoint carrying the revision forward; originals renamed `*.migrated`; project-level sidecar files absorbed.
5. `1b1864a` — Unify run transcripts onto shared conversation records. `attractor-runtime` writers rebuilt on the shared `Transcript`/`TranscriptSegment` with typed `BoundaryMeta`; shared identity derivation; legacy `entries`-shape files read compatibly; the Runs UI renders LLM content exclusively from the transcript and imports the chat segment types instead of duplicating them.
6. `a5caf5e` — Extract Codex runtime continuity into a runtime-session sidecar. `runtime-session.json` written best-effort on thread observation, tombstoned on resume failure; prompt-time continuity reads the sidecar with a turn-scan fallback; transcript turns keep thread ids as provenance only.
7. (this change) — Remove legacy conversation scaffolding and finalize record-model docs. `write_snapshot` and dead legacy helpers deleted (migration tests seed raw legacy files by hand); the continuity turn-scan fallback made self-healing; specs and this change request updated to the implemented model.

## Notable deviations from the initial plan

- Settings updates now journal (previously they wrote state without journal events).
- Turn completions no longer append a trailing full `conversation_snapshot` journal line; committed entries are coalesced final render values (one upsert per turn/segment id per commit). Metadata/artifact-level changes still journal one snapshot entry.
- `persist_snapshot_with_events` (the interim commit path) was deleted in phase 4 rather than the final phase: after the file split it wrote files production reads ignore, and it had no production callers since phase 2.
- The run transcript wire evolved (`/pipelines/{id}/transcript` serves shared snake_case segments, boundaries included) instead of projecting the old camelCase `entries` shape — the in-repo frontend is the only consumer, and a byte-compatible projection would have kept three copies of one model alive.
- Context-compaction notices in run transcripts coalesce into one segment via shared identity (previously one notice per event).
- The continuity turn-scan fallback was kept (self-healing: a successful scan materializes `runtime-session.json`) instead of deleted — deleting it would have silently broken thread resume for every conversation created before the sidecar existed.
- The keyed per-provider session path (`sessions/<provider>-<hash>.json`) and other never-called path helpers were removed outright.

## Validation

- Full gate green after every phase and at completion: 107 Rust test suites, 363 frontend tests, frontend build.
- New contract suites: `conversation_commit_contracts` (revision monotonicity, identity rebase incl. the artifact-vs-streaming-turn interleaving, order stability, rejects, projection round-trip, transient-never-journaled), `conversation_migration_contracts` (migrate-on-first-read, revision continuity + checkpoint replay, unsupported-schema untouched, idempotency), `conversation_runtime_session_contracts`, `transcript_contracts` (attractor-runtime shared coalescing + legacy read compat), SSE stream-delta contracts, and frontend transient-apply cases.
- Not exercised here: interactive end-to-end verification against a live Codex backend (streaming feel, reconnect mid-turn, thread resume across restart). Recommended as a manual smoke pass.
