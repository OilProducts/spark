---
type: agent_task
title: Validate Active Item State
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - context.item.attempts
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked State
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: review_current
  label: Review
  condition: outcome=success
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Repair Queue
  condition: outcome=fail && preferred_label=Replan
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Repair Queue Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
---

Read the current milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json. Treat context.item.id as the authoritative live binding for the active item once work on an item has begun. Confirm state.json.current_item_id equals context.item.id, the matching item exists exactly once in the items array, that item is status=in_progress, and no other item is status=in_progress. Repair purely mechanical drift directly, for example rewriting current_item_id to match when context.item.id already names the sole in_progress item. If context.item.id is missing, state.json points at a different item, multiple items are in_progress, the matching item is missing, or a downstream item advanced despite attempts=0 or unmet dependencies, return outcome fail with preferred_label Replan so Validate Item Plan can repair the queue from the evidence. Respond with ONLY a JSON status envelope. Return outcome success only when the active-item live binding is coherent and the durable ledger agrees with it.
