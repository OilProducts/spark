## F03 — Accessible node-inspector fields

### Architecture decision

Keep the current organization:

- `Sidebar` coordinates selection, property updates, validation, and saving.
- `NodeInspectorPanel` presents fields according to node capabilities.
- Shared editors own the presentation of context keys and extension attributes.

These boundaries fit the current responsibilities. The defect is incomplete label/control wiring, not a need for different domain concepts. Retain explicit JSX; do not introduce a form schema, field registry, new state layer, or generic inspector framework.

### Implementation

- Associate every visible node-inspector field label with its control using `htmlFor` and `id`, covering basic, conditional, and advanced fields—not only the nine reported in the audit.
- Generate stable, instance-specific IDs with React `useId` plus field suffixes. Preserve existing test IDs and correctly labeled checkbox behavior.
- Fix `ContextKeyListEditor` internally so both callers benefit. Use **Reads Context** and **Writes Context** as the respective visible, associated labels, replacing the redundant generic “Context Keys” label. Keep its existing props and edit callbacks.
- Connect existing field-specific help, warnings, and rendered validation messages through `aria-describedby`. Set `aria-invalid` for actual errors, not warnings, and remove obsolete references when messages disappear.
- Give node diagnostic output an addressable wrapper within the panel; retain the existing diagnostic callback and validation logic.
- Preserve field ordering, visibility rules, values, draft handling, and save behavior. No public API, storage, or runtime changes.

### Verification

Extend the existing inspector authoring tests to:

- Locate controls by accessible role/name and edit representative agent, tool, parallel, and subflow fields.
- Cover both conditional parallel thresholds and expanded advanced settings.
- Verify Reads/Writes fields have distinct names and call the correct callbacks.
- Verify description/error associations, error clearing, and warning-only states.
- Check label associations after changing selected nodes and unique IDs across repeated context-editor instances.
- Retain existing extension-field and checkbox coverage.

Run the affected editor tests, then `just test`. Perform a keyboard and native VoiceOver check on the Implement subflow node, confirming named fields and label-click focus; report any unavailable manual verification explicitly.

### Scope and follow-up

This change covers the node inspector and its context-key editor. Graph/edge inspector accessibility, wording changes from F16, and visual restructuring remain separate work.

Splitting the large panel into capability sections can be reconsidered if those sections acquire independent behavior. Its size alone does not justify that refactor for this fix.
