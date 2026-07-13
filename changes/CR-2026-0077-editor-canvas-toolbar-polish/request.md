# Editor Canvas Toolbar Polish

## Summary

Replace the loose row of canvas buttons with one compact, grouped toolbar that has clear hierarchy, uniform sizing, and responsive wrapping. Preserve every existing action and its behavior; this is a presentation and accessibility refactor.

## Implementation Changes

- Extract the canvas controls from `Editor.tsx` into a focused internal toolbar component with explicit props for mode, child-flow view, layout actions, node creation, Run state, and callbacks.
- Render one translucent surface with uniform 32px controls and three separated groups:
  - View: `Structured | YAML` and `Parent | Expanded`
  - Layout: Arrange and immediate Reset actions using Lucide icons, concise accessible names, and tooltips
  - Actions: `+ Node` as a restrained secondary action and `Run` as the only primary action
- Preserve current visibility rules:
  - Child-flow controls appear only in structured mode.
  - Arrange, Reset, and Add Node are hidden in expanded read-only mode.
  - Run remains available in structured mode and preserves its disabled reason.
  - Raw/structured handoff behavior and pending-state disabling remain unchanged.
- Improve semantics:
  - Mark segmented choices with `aria-pressed`.
  - Give each group an accessible label.
  - Keep icon controls keyboard-focusable with stable accessible names.
  - Preserve the Run disabled explanation.
- Make the toolbar container wrap complete groups at narrow widths rather than clipping individual controls. Keep its current top-left canvas position and ensure wrapped rows do not obscure React Flow controls or the expanded-preview notice.
- Retain immediate Reset behavior without confirmation, but style it as a low-emphasis layout utility rather than a primary action.
- Leave React Flow’s built-in zoom controls and minimap unchanged.

## Test Plan

- Update editor tests to interact through accessible toolbar names while preserving coverage for:
  - Structured/YAML switching and raw handoff failures.
  - Parent/Expanded switching and read-only behavior.
  - Arrange and immediate Reset filesystem/local-storage effects.
  - Add Node visibility and behavior.
  - Run enablement, disabled reason, and launch-panel opening.
- Add focused toolbar behavior tests for active `aria-pressed` states, keyboard-accessible icon actions, group visibility, and wrapped-container rendering.
- Avoid pixel snapshots and class-string assertions; test observable labels, state, callbacks, and responsive availability.
- Run the full validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions

- The visual direction is a compact grouped bar, not an icon-only toolbar.
- Short visible labels are `YAML`, `Parent`, `Expanded`, `+ Node`, and `Run`; Arrange and Reset use icons with tooltips and accessible names.
- Reset remains immediate and behavior-compatible.
- Narrow layouts wrap complete groups onto another row; no overflow menu or horizontal scrolling is introduced.
- No backend, flow-definition, persistence, or public API changes are required.
