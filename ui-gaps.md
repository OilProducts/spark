# UI Gaps Backlog (Attractor)

This file captures UI features implied by `attractor-spec.md` that are not yet implemented, along with a working Definition of Done. It reflects current UI decisions and open questions.

**Context**
The UI currently covers basic node editing, flow loading/saving, and run initiation. Several spec-backed features are missing or only partially represented.

**Decisions Captured From User**
1. Human gate UI should be indicated in the flow list and in the graph during execution. Use a node-expanding panel for input.
2. Edge attributes are user-editable.
3. Graph-level settings should live in a top-bar drawer within the graph canvas, not the app shell. Model + path inputs should move there.
4. Advanced node fields should be behind a toggle in the node edit interface.
5. Validation feedback should be inline annotations.
6. Run/Pause/Resume/Cancel controls should live in a footer inside the graph canvas.
7. Manager loop (`house`) needs clarification.
8. Backlog file path is this file.

**Definition of Done (applies to each item unless overridden)**
1. UI behavior matches `attractor-spec.md` for the feature in scope.
2. UX is consistent with existing shadcn token usage and current layout conventions.
3. UI updates persist correctly to `.dot` via the current API.
4. Errors are handled visibly and do not silently fail.
5. Frontend lint and build pass.

**Backlog Items**
1. Human gate UI end-to-end. **DONE**
DoD: Flow list shows a clear human-input-needed indicator; executing graph highlights/expands the gate node; user can choose an option; selection routes execution; handler no longer auto-approves for active UI runs.
Notes: Implemented node-expanding panel, flow list indicator, and a web interviewer path. Auto-approve only when no websocket clients are connected.
Open question: None (node-expanding panel chosen).

2. Live node status updates during execution. **DONE**
DoD: Nodes reflect running/success/fail based on `/ws` state messages, and UI resets appropriately between runs.

3. Edge attributes: display and editing.
DoD: User can view edge attributes (label/condition/weight/etc.) and edit them if required; changes persist to DOT; selection is obvious.
Open question: None (edge attributes are editable).

4. Graph settings drawer inside canvas.
DoD: A graph-scoped settings drawer exists inside the canvas top bar, containing model/path inputs and graph-level attributes (goal, label, stylesheet, retry targets, default retry, default fidelity).

5. Advanced node attributes UI.
DoD: Node editor includes a collapsed Advanced section for `type`, `max_retries`, `goal_gate`, `retry_target`, `fallback_retry_target`, `fidelity`, `thread_id`, `class`, `timeout`, `llm_model`, `llm_provider`, `reasoning_effort`, `auto_status`, `allow_partial`.

6. Inline validation and diagnostics.
DoD: Parse/validation errors from `/preview` surface inline on nodes/edges (badges + list), and block execution when errors exist.

7. Run controls in canvas footer.
DoD: Run/Pause/Resume/Cancel controls appear inside the canvas footer; controls reflect current run state and are disabled/enabled correctly.

8. Manager loop handler (`house`) UI exposure.
DoD: If handler exists, UI allows selecting `house` (manager loop) with required fields; if not, remove it from UI options and document as not supported.
Open question: Defer manager loop (`house`) UI until handler exists.

9. Model stylesheet editing/inspection.
DoD: UI for viewing/editing `model_stylesheet` with basic lint feedback; stylesheet persists to DOT graph attributes.

10. Fan-in LLM evaluation support.
DoD: If fan-in node has a prompt, use LLM-based ranking; UI exposes results or selected candidate.

11. Checkpoint/run artifact visibility.
DoD: UI shows current run metadata (working dir, model, status), last error, and exposes artifact links or summaries (prompt/response/status).
