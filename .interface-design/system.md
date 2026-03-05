# Interface System

## Anchors
- Human: operators moving between project context, AI conversation, and workflow traceability.
- Task: stay in one home workspace long enough to switch projects, review events, and drive project-scoped chat without hunting for context.
- Feel: quiet workstation, not a marketing page; dense enough for real work, calm enough to scan.

## Direction
- Dominant direction: neutral utility surfaces with subtle shadows and bordered cards.
- Typography: compact hierarchy with small operational labels and restrained headings.
- Depth strategy: subtle-shadows only. Borders define structure first; shadow is secondary.
- Spacing grid: 4px/8px rhythm.

## Layout Rules
- Desktop app surfaces should prefer full-height work areas over document-style vertical stacks when the task is ongoing.
- In the home workspace, the main content is a two-pane shell: project sidebar on the left, project conversation workspace on the right.
- Major panes own their own scroll regions instead of forcing the whole page to scroll.
- The home sidebar is a split stack: quick-switch context above, workflow event log below.
- Desktop sidebar stack uses a visible draggable separator with keyboard support. Narrow layouts collapse to stacked cards and remove the separator.
- Chat composers should stay anchored to the bottom of their working card when the surrounding surface is full-height.

## Surface Patterns
- Card chrome stays consistent: `border`, `bg-card`, `shadow-sm`, rounded corners.
- Dense operational lists may scroll internally when they live inside a full-height pane.
- Resize affordances should be understated but obvious on hover/focus and use the correct resize cursor.
