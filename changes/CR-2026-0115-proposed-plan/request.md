## Tasks — Visual hierarchy and readable task details

### Summary

Improve all six design areas while retaining the simplified task model: title, description, stage, and archived status. Make lanes visually distinct, cards easy to scan, and existing tasks open for reading before editing.

Build on the existing horizontal board, selected-card highlight, side panel, and project-scoped draft handling. No backend changes, new dependencies, or restoration of removed task fields.

### Board, lanes, and cards

- Keep all six stages in one horizontal row. Use lanes at least 15rem wide, distributing spare width evenly and scrolling horizontally when necessary.
- Give lanes equal full-height muted backgrounds, subtle borders, rounded corners, and consistent padding. Keep their headers visible while each lane’s cards scroll vertically.
- Show a compact stage heading and count of matching tasks. Empty lanes retain their height and show a quiet “No tasks” or “No matches” message.
- Use theme tokens for lane/card contrast in light and dark modes. Reserve accent color for selection and focus; avoid six competing stage colors.
- Lead each card with a semibold title, wrapping to three lines, followed by an optional two-line description excerpt in muted text. Show an Archived badge when applicable. Full content remains available in task details.
- Preserve server ordering. Keep cards keyboard-operable, with distinct hover, focus, and selected states.

### Toolbar and selection

- Keep Create task as the primary action.
- Add a labeled title-search input with a clear action. Filter locally, case-insensitively, combining search with Show archived.
- Present Show archived as a toggle button with `aria-pressed`. Replace the prominent Refresh button with a labeled icon button and tooltip.
- Counts reflect filtered cards; filtering does not close an open task or discard its draft.
- Preserve board and lane scroll positions when opening/closing details. Keep the selected card visibly highlighted whenever it is in view.
- Retain the existing 28rem side panel on wide windows and full-width detail replacement at the existing narrow breakpoint. Global navigation remains usable.

### Read and edit modes

- Open saved tasks in a readable detail view: title, stage, description with preserved paragraphs/line breaks, and an Archived indicator when applicable.
- Provide explicit Edit and Close actions. Put activity, task ID, and revision in a collapsed secondary section.
- Allow immediate stage changes from read mode through the existing revision-checked PATCH endpoint, sending only the stage. Show saving/error feedback and preserve the prior value on failure.
- Refresh read mode from incoming server records. If a stage change conflicts, refresh and require the user to retry rather than silently overwrite.
- Creation opens the existing compact form. Edit opens that form populated from the saved task. Successful saves return to read mode.
- If an unsaved draft exists, reopening that task resumes edit mode and shows Unsaved changes.
- Close, Escape, navigation, and project changes preserve drafts. Explicit discard returns existing tasks to the latest saved read view; discarding a new task closes creation.
- Preserve conflict reconciliation, in-flight save ownership, and disabled editing/dismissal during saves. Quick stage changes are available only in read mode, so they cannot overwrite draft edits.
- Focus the detail heading on opening, Title on creation/editing, and the originating card/action on closing, with the board heading as fallback.

### Architecture and verification

Keep fetching, mutations, selection, and drafts in the existing project controller. Keep `TaskEditor` focused on the edit form; add a small task-detail presentation component for read mode. Reuse shared controls and existing styles rather than introduce a board framework.

Extend existing tests for search/counts, archive filtering, read/edit transitions, draft restoration, partial stage updates, failures/conflicts, project switches, and keyboard focus.

Use the existing browser smoke suite with populated and empty lanes, long content, archived tasks, and both themes. Capture and inspect before/after screenshots at wide and narrow sizes, with details open and closed. Verify readable lane widths, horizontal scrolling, persistent headers, reachable actions, and visible selection. Run affected tests and `just test`.

### Scope

Priority, next action, attention flags, acceptance criteria, and resource links remain absent under the chosen simplified model. Description excerpts provide the secondary card content; Show archived is the available filter alongside title search. Drag-and-drop and custom ordering remain deferred.
