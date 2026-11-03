---
type: agent_task
title: Validate Active Item State
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - context.item.attempts
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  replan:
    to: validate_item_plan
    description: The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning.
  unfinished:
    to: validate_active_item_state
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  verified:
    to: review_current
    description: The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed.
---

Read the current milestone state in .spark/state/spec-implementation/current/milestones/<current milestone id>/state.json. Treat context.item.id as the authoritative live binding for the active item once work on an item has begun. Confirm state.json.current_item_id equals context.item.id, the matching item exists exactly once in the items array, that item is status=in_progress, and no other item is status=in_progress. Repair purely mechanical drift directly, for example rewriting current_item_id to match when context.item.id already names the sole in_progress item. If context.item.id is missing, state.json points at a different item, multiple items are in_progress, the matching item is missing, or a downstream item advanced despite attempts=0 or unmet dependencies, choose `replan` so Validate Item Plan can repair the queue from the evidence. Choose `verified` only when the active-item live binding is coherent and the durable ledger agrees with it. Respond with ONLY a JSON decision envelope.
