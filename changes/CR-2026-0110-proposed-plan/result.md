## Shipped

Added project-scoped durable tasks, a workspace task service and collection/item APIs, `spark task list|get|create|update`, and the active project's Tasks board and detail editor.

Task records and attributed activity history share an atomic storage transaction protected by a project file lock. Revision checks reject stale writers. Fixed lifecycle stages remain independent of run outcomes; completion requires a note, and tasks support reopening, archival, priority, next actions, blockers, questions, and validated conversation, repository-file, and run references.

Conversation commits now hold a per-conversation file lock across revision allocation and publication. This fixes the reader/writer transcript-revision race exposed by the full gate and protects task link validation alongside live conversation writes.

The board supports keyboard-accessible editing, priority ordering, archived/attention filters, refresh and visible-only polling. Drafts survive refresh, navigation, and project changes; conflicts expose server changes for explicit reconciliation. Run statuses and descendant questions use existing runtime interfaces and answer storage.

Task run navigation selects the requested run in the Runs view's current scope. Both Open run and descendant Answer questions replace a previous selection in active-project and All projects modes. Regression checks use the effective Runs selection selector; both All projects cases failed before the shared-handler fix and passed afterward. Existing question-answer storage and approvals are preserved.

Optional conversation run-request task references survive approval and launch in durable run metadata. Idempotent replay repairs interrupted task associations, preserving manual unlinking. Bundled assistant guidance documents structured file/stdin commands and task maintenance without launching work or bypassing approvals.

## Verification

Added service checks for persistence, atomic history, concurrent revisions, invalid links, project isolation, lifecycle, manual/multiple-run tasks, conversation links, approval/launch recovery, and descendant questions. Added CLI-to-real-HTTP interoperability checks and UI checks for accessible editing, attention routing, draft preservation, polling, and concurrent changes.

Validation passed: `RUST_TEST_THREADS=4 just test` completed successfully (836 Rust tests and 464 frontend tests passed, plus formatting and the production build). Focused task, CLI/HTTP, UI, and concurrent-conversation checks also passed. An initial overlapping gate attempt hit an adapter timeout; a later gate exposed the conversation revision race fixed above. No tests were weakened or excluded for this delivery.

Navigation review validation: the affected Tasks, Runs, and selection-selector suites passed (38 tests). A fresh, unmodified `just test` gate passed, including all Rust suites, 467 frontend tests, formatting, and the production build. `git diff --check` passed.

The excluded scheduling, coordination, imports, account/permission systems, destructive task deletion, drag-and-drop, custom ordering, and dedicated principles editor remain outside this delivery.
