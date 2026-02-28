# UI Parity Risk Report

Checklist item: [1.1-02]

Date: 2026-02-28

## Scope

This report scores parity risks where current UI surfaces can lose behavior or hide required configuration from users, based on [`ui-spec.md`](/Users/chris/tinker/sparkspawn/ui-spec.md) and the raw-DOT gap inventory in [`ui-raw-dot-required-config.md`](/Users/chris/tinker/sparkspawn/ui-raw-dot-required-config.md).

## Behavior-Loss Failure Modes

| Failure mode | Trigger in current UI | User-visible impact | Severity | Mitigation direction |
| --- | --- | --- | --- | --- |
| Save/re-save drops `stack.child_dotfile` or `stack.child_workdir` because no structured graph fields exist. | Author edits in structured UI and saves a flow that previously relied on child manager graph attrs. | Manager-loop child pipeline execution runs with missing path/workdir context; runtime behavior diverges from prior DOT. | High | Add graph attr controls and round-trip serialization tests for both `stack.child_*` attrs. |
| Save/re-save drops `manager.actions` and other manager attrs because manager-loop field group is absent. | Author modifies a manager node through current inspector fields and saves. | Manager supervisory policy changes silently; loop can stall, overrun, or terminate incorrectly. | High | Add full manager-loop authoring controls and handler-specific round-trip tests. |
| Save/re-save omits `human.default_choice` because authoring UI cannot set timeout default. | Author updates wait-human node in UI and saves. | Timeout path semantics differ from intended reviewer decision default, changing branch behavior. | Medium | Add explicit `human.default_choice` input plus validation and run-time visibility. |
| Subgraph/default scopes (`subgraph`, `node[...] defaults`, `edge[...] defaults`) are not representable in structured editing. | Author reworks nodes/edges in canvas/inspector and saves. | Inheritance and scoped defaults are flattened or lost, causing broad behavior drift across nodes/edges. | High | Add first-class subgraph/default authoring and serialization parity tests. |

## Hidden-Config Failure Modes

| Failure mode | Hidden required config | Discovery signal in current UI | Severity | Mitigation direction |
| --- | --- | --- | --- | --- |
| Tool hook requirements are invisible in graph settings. | `tool_hooks.pre`, `tool_hooks.post` | No dedicated graph controls or warning badge indicating hook config exists only in DOT. | High | Add tool-hook editor fields and show non-empty hook indicators in graph settings summary. |
| Manager-loop behavior contract is hidden behind raw DOT-only attrs. | `manager.poll_interval`, `manager.max_cycles`, `manager.stop_condition`, `manager.actions` | Node UI allows generic `type` override but does not expose required manager behavior attrs. | High | Add manager-loop inspector section with typed fields and requiredness hints. |
| Human timeout fallback behavior is hidden in authoring surface. | `human.default_choice` | Runtime human-answer UI exists, but authoring UI lacks default-choice configuration cues. | Medium | Add wait-human advanced section for timeout/default semantics and preview in node summary. |
| Scoped structure contract is hidden from graph authoring. | `subgraph`, `node[...] defaults`, `edge[...] defaults` | Canvas and inspector show only concrete nodes/edges with no scoped defaults surface. | High | Add scoped authoring panel and indicators that a flow contains defaults/subgraphs. |

## Notes

- Severity is scored for behavior correctness and likelihood of silent misconfiguration.
- This report complements the raw-DOT inventory and provides a parity risk priority baseline for implementation sequencing.
