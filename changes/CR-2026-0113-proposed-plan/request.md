## F05 — Consistent task controls and a compact editor

### Summary and architecture

Replace the form below the board with a non-modal side panel. Make basic task capture quick and existing tasks easier to review using Spark’s established controls.

Keep project sessions, drafts, polling, saves, and conflict handling in the existing task controller. Extract a task-editor component for presentation and callbacks. No generic form framework, new dependency, backend changes, or storage migration.

### Layout and controls

- On wide windows, show a roughly 28rem editor beside the board, with independently scrolling content. Let board columns wrap to their available width.
- At the existing narrow-viewport breakpoint, show the editor full-width in place of the board. Closing restores the board; global navigation remains available.
- Reuse `Button`, `Input`, `Textarea`, `NativeSelect`, `Checkbox`, and associated labels. Style Create task and Save task as primary actions; Refresh and Close as secondary.
- Use a single-line Title input. Initially show Title, Outcome and decisions, Next action, Stage, and Priority. Only Title is required for ordinary creation.
- Put acceptance criteria, blockers/questions, archival, and raw relationship editing in labeled expandable sections. Automatically reveal existing blocker/question content.
- Keep Note / completion evidence available in a separate section, automatically expanded and required when moving to Done.
- Keep the panel header and Save/Close footer visible while its body scrolls. Use “New task” or the saved task title as the heading, with ID/revision secondary.
- Show related resources only when populated and activity only for saved tasks. Preserve existing navigation and relationship editing.

### Drafts, dismissal, and saving

- Close and Escape preserve the draft without saving or prompting. Show concise guidance that drafts remain available during this workspace session.
- Reopening the task—or Create task for a new draft—restores its fields and note. Preserve drafts across card selection, tab navigation, project changes, and board refresh.
- Show an Unsaved changes indicator based on differences from the editing baseline, including notes.
- Provide a separate **Discard changes** action. For a new task, it clears the draft and closes the panel. For an existing task, it reloads the latest available record and clears the note.
- Successful saves keep the saved task open, update its baseline, and clear the unsaved indicator. Failures preserve all input.
- Retain explicit conflict reconciliation and disabled saving until a newer revision is reconciled. Keep conflict details visible inside the editor.
- Disable editing and dismissal during saving to avoid losing or misapplying an in-flight response.
- Focus Title on creation and the editor heading when opening a saved task. Restore focus to the originating card/Create action on close, falling back to the board heading if necessary. Handle Escape within the editor only after any nested control has handled it.

### Verification

Extend existing Tasks tests to cover title-only creation, shared accessible controls, close/reopen, explicit discard, successful and failed saves, unsaved indicators, completion evidence, and conflict reconciliation.

Retain regression coverage for project-scoped drafts, visible-only polling, archival/filtering, and linked-run/question navigation.

Check a populated board at wide and narrow sizes: the editor opens immediately, primary actions remain reachable, expanded sections scroll properly, and keyboard focus behaves correctly. Run affected frontend tests and the full `just test` gate.

### Scope

This addresses F05. Searchable relationship selectors (F06), broader empty-state design (F14), drag-and-drop, and task automation remain separate changes. Draft preservation remains session-based; restart persistence is not added.
