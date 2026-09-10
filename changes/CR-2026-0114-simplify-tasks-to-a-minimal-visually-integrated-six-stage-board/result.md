# CR-2026-0114 result

Implemented the minimal Tasks board across the service, API contract, CLI, frontend, bundled guidance, and tests. Records contain title, optional plain-text description, one of the six manual stages (Backlog by default), and archive status, plus identity, timestamps, revision, and attributed activity. Done requires no note; optional API notes remain supported.

Removed unused fields, relationships, resource associations, conversation provenance, attention data, task/run references, CLI task launch flags, launch metadata injection, and reconciliation. Listing returns only `{ "tasks": [...] }`, ordered by creation time then ID, without reading run state. Existing atomic storage transactions remain unchanged; no migration or compatibility layer was added.

The board has title-only cards and Show archived. Create/edit panes use shared controls and theme tokens, with separate archive/restore actions that preserve unsaved edits. Activity starts collapsed and shows readable field changes, actor, and timestamp. Revision conflicts preserve drafts and offer readable reconciliation. Project switching, pending saves, keyboard access, Escape, and focus restoration retain their protections.

Validation:

- Affected Rust service, HTTP, CLI, and ordinary conversation approval/launch tests passed. Coverage includes persistence, attribution, project isolation, stale revisions, rejected removed fields/flags, deterministic ordering, and independence from corrupt run metadata.
- Task UI tests: 21 passed, covering all stages, title-only creation, optional description, archive/restore, drafts, pending saves, conflicts, and accessibility.
- Frontend build and repository `just test` passed (493 frontend tests).
- All six browser checks passed. Browser checks cover light/dark at 1440, 1024, and 390 pixels, with captured Runs comparisons, empty/populated boards, long titles, loading/errors, keyboard focus, create/edit, conflicts, and activity. Visual review corrected wrapped stage columns and low-contrast dark error text. The board scrolls horizontally when necessary; the editor replaces it at narrow widths while retaining accessible actions.

No dependencies, scheduler, execution controls, automatic stage advancement, drag-and-drop, destructive deletion, or other-tab redesign were introduced.
