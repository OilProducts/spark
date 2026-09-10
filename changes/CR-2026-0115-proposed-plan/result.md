Implemented the six-stage board with 15rem minimum lanes, equal themed backgrounds, fixed lane headers, independent card scrolling, filtered counts, empty states, title/description excerpts, archived badges, and distinct selection/focus states. Added local title search, clearing, an accessible Show archived toggle, and a refresh icon with tooltip. Server order is retained.

Saved tasks open in the new read presentation with preserved description formatting, archived status, collapsed activity/ID/revision, and revision-checked stage-only updates. Errors retain the prior stage; conflicts refresh and require an explicit retry. The existing project controller owns records, selection, mutations, and drafts. TaskEditor handles creation/editing, successful saves return to reading, and explicit discard restores the saved view or closes creation. Draft ownership, reconciliation, focus restoration, and save-time locks remain covered across project switches. Filtering and detail transitions preserve board/lane scroll positions.

Validation:

- `just test`: passed (Rust workspace tests, 60 frontend test files / 497 tests, production build).
- Tasks unit tests: 25 passed, including partial PATCH bodies, failures/conflicts, drafts, archive/search filtering, read/edit focus, and deferred saves across projects.
- Tasks browser smoke: 6 passed at 1440, 1024, and 390 pixels in both themes. Assertions cover minimum lane width, horizontal overflow, persistent headers, scroll restoration through filtering/open/close, visible selection, reachable actions, and keyboard focus. Baseline Tasks smoke also passed all 6 cases.
- Full browser smoke: 22 passed, 7 failed outside Tasks. `editor-diagnostics.spec.ts` fails at lines 30, 95, 288, 553, and 615 on missing canvas/editor controls; `editor-save.spec.ts:17` times out locating Raw YAML; `rust-product-shell.spec.ts:30` expects `/var/...` while the server returns the canonical macOS `/private/var/...` path. These files and their product paths were not changed. Full-suite log: `/tmp/cr0115-smoke-final.log`.

Captured and inspected before/after screenshots. The [screenshots](screenshots/) directory retains light/dark, wide/narrow board and detail views, empty lanes, long content, and archived tasks. Inspection confirmed readable 240px lanes, full-height empty lanes, preserved paragraph spacing, visible selected cards, and reachable detail actions. Scrolled read screenshots deliberately retain the board offset to demonstrate scroll preservation.

| View | Before board | After board | Before details | After details |
| --- | --- | --- | --- | --- |
| Light, 1440px | [PNG](screenshots/before-light-1440-board-focus.png) | [PNG](screenshots/after-light-1440-populated.png) | [PNG](screenshots/before-light-1440-editor.png) | [PNG](screenshots/after-light-1440-read.png) |
| Dark, 1440px | [PNG](screenshots/before-dark-1440-board-focus.png) | [PNG](screenshots/after-dark-1440-populated.png) | [PNG](screenshots/before-dark-1440-editor.png) | [PNG](screenshots/after-dark-1440-read.png) |
| Light, 390px | [PNG](screenshots/before-light-390-board-focus.png) | [PNG](screenshots/after-light-390-populated.png) | [PNG](screenshots/before-light-390-editor.png) | [PNG](screenshots/after-light-390-read.png) |
| Dark, 390px | [PNG](screenshots/before-dark-390-board-focus.png) | [PNG](screenshots/after-dark-390-populated.png) | [PNG](screenshots/before-dark-390-editor.png) | [PNG](screenshots/after-dark-390-read.png) |

No backend changes or new dependencies. Removed task fields remain absent; drag-and-drop and custom ordering remain deferred.
