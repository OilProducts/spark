---
type: agent_task
title: Gate Item Completion
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
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
- to: mark_current_done
  label: Validated
  condition: outcome=success
  extensions:
    loop_restart: false
- to: prepare_validation
  label: Revalidate
  condition: outcome=fail && preferred_label=Revalidate
  extensions:
    loop_restart: false
- to: prepare_validation
  label: Revalidate Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked State Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read the current milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json and .spark/spec-implementation/current/validation-result.json. Treat context.item.id as the authoritative live binding for the active item. Confirm state.json.current_item_id and .spark/spec-implementation/current/validation-result.json.item_id both equal context.item.id, that the matching state item exists exactly once and is status=in_progress, and that validation status=pass with all required suites passing. Treat missing validation-result.json, stale or mismatched item ids, missing active state in the durable ledger, blocked validation, failed validation, skipped required suites, or blocked required suites as non-pass. If the live binding and durable ledger disagree, reconcile the ledger to context.item.id from the evidence (make that item the sole in_progress item and set current_item_id to it) before judging validation, and treat a validation-result.json that names a different item as missing validation evidence. If queue identity is coherent but validation evidence is missing or non-passing for context.item.id, return outcome fail with preferred_label Revalidate and write context.review.summary, context.review.required_changes, and context.review.blockers describing what validation evidence is missing or non-passing. Respond with ONLY a JSON status envelope. Return outcome success only when the active item has a passing validation result and the live binding agrees with the durable ledger.
