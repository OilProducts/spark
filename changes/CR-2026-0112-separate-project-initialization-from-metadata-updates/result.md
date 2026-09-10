# CR-2026-0112 result

Implemented idempotent project initialization and explicit registration. Valid metadata is left untouched while missing directories are recreated. Missing or invalid metadata is initialized/repaired with recoverable custom names, timestamps, and optional settings preserved. Legacy absent optional fields retain their defaults without requiring a rewrite.

`ProjectRegistry::register_project` refreshes `last_opened_at`; workspace registration now calls it explicitly. Project reads resolve paths and read without initialization. Updates still initialize missing projects and preserve unspecified fields and existing opening timestamps. Conversation and task initialization, flow-launch profile lookup, frontend activity updates, HTTP interfaces, persisted field names, and provider persistence remain unchanged. No dependencies or concurrency redesign were added.

Contract coverage includes directory recreation, pure reads, registration and update preservation, repair/defaults, empty-path validation, unregistered conversation/task operations, native execution-profile fallback, and provider-event persistence with unchanged metadata bytes, modification time, and Unix device/inode identity. Timestamp-sensitive checks use fixed historical values without sleeps.

Validation:

- Storage contracts: 51 passed.
- Workspace contracts: 53 passed, 1 existing ignored.
- Unchanged 50,000-event test: passed before and after. Test harness runtimes were **354.74 s before** and **8.65 s after**; external elapsed times were **354.76 s** and **9.09 s** respectively. Each measurement invoked the same filtered debug test executable directly after a completed `cargo test -p spark-workspace --test process_contracts --no-run` build, on the same machine/worktree, with no competing build or test suite. Compilation was excluded. No wall-clock assertion was added.
- `just test`: passed (format check, full Rust workspace with all features and documentation tests, 478 frontend unit tests across 60 files, and production frontend build). The existing `live_provider_event_write_failure_prevents_transcript_commit` contract passed.
