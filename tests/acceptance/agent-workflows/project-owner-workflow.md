# Project Owner Workflow

## Goal

Verify that a project owner can work inside a single project-scoped Home workflow from conversation through spec-edit review and execution-card review.

## Preconditions

1. An active project is selected.
2. Project chat is available and backed by the chat runtime.
3. The project has a usable flow for follow-on execution when needed.

## Workflow

1. Open the Home area.
2. Select or create a project conversation thread.
3. Send a project-scoped message in chat.
4. Observe streaming assistant activity and any inline tool calls.
5. Review any proposed spec-edit card that appears.
6. Approve or reject the spec-edit card explicitly.
7. If approved, observe workflow progress in the workflow event log.
8. Review the resulting execution card when it appears in the conversation timeline.
9. Approve, reject, or request revision on the execution card.
10. If approved and supported by the selected flow, continue toward build execution.

## Expected Outcomes

- Threaded project chat remains scoped to the active project.
- Tool activity and assistant output are visible in the conversation timeline.
- Spec edits are explicit review artifacts, not silent mutations.
- Execution planning returns as an execution card in the same conversation context.
- Workflow/system progress stays in the event log rather than polluting the chat timeline.
