---
id: CR-2026-0057-decouple-conversation-turn-acceptance-from-assistant-completion
title: Decouple Conversation Turn Acceptance From Assistant Completion
status: completed
type: feature
changelog: public
---

## Summary

`POST /workspace/api/conversations/{conversation_id}/turns` now represents durable turn acceptance: the service returns a snapshot containing the completed user turn and pending assistant turn while assistant execution continues in the background. Post-acceptance backend failures are persisted as failed assistant turns and surfaced through conversation progress events instead of turning the accepted POST into a failure.

The project chat UI no longer inserts an optimistic user timeline row. Sending clears the draft, disables the composer with a short `Sending...` state, and waits for the started snapshot or live events to populate the timeline through the normal conversation cache.

## Validation

- `uv run pytest -q` passed: 1955 passed, 26 skipped.

## Shipped Changes

- Backend conversation service and workspace API behavior: `ProjectChatService.start_turn` prepares and persists the accepted user/pending assistant turn pair, starts background assistant completion, and keeps failure/progress events durable and replayable.
- Frontend project conversation state: replaced `optimisticSend` with `pendingSend`, removed optimistic timeline message construction, and used pending state only for composer disabled/label behavior.
- Frontend controller/composer hooks: record the starting conversation revision, clear pending send state when a newer snapshot/event arrives, and treat the POST response as the started snapshot.
- Tests: updated project panel/view-model expectations for no optimistic duplicate and added API/service regression coverage for started snapshots plus background failure publishing.
