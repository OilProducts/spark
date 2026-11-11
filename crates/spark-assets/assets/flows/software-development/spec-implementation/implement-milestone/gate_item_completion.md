---
type: agent_task
title: Gate Item Completion
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - internal.project_work_dir
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  revalidate:
    to: prepare_validation
    description: The active item lacks current passing validation evidence; preserve its incomplete state and record precisely what must be rerun.
  unfinished:
    to: gate_item_completion
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  validated:
    to: mark_current_done
    description: The current queue or completion state passes this node’s identity, dependency, coverage and validation checks after any permitted repairs.
---

Read the current milestone state in <internal.project_work_dir>/spec-implementation/current/milestones/<current milestone id>/state.json and <internal.project_work_dir>/spec-implementation/current/validation-result.json. Treat context.item.id as the authoritative live binding for the active item. Confirm state.json.current_item_id and <internal.project_work_dir>/spec-implementation/current/validation-result.json.item_id both equal context.item.id, that the matching state item exists exactly once and is status=in_progress, and that validation status=pass with all required suites passing. Treat missing validation-result.json, stale or mismatched item ids, missing active state in the durable ledger, blocked validation, failed validation, skipped required suites, or blocked required suites as non-pass. If the live binding and durable ledger disagree, reconcile the ledger to context.item.id from the evidence (make that item the sole in_progress item and set current_item_id to it) before judging validation, and treat a validation-result.json that names a different item as missing validation evidence. If queue identity is coherent but validation evidence is missing or non-passing for context.item.id, choose `revalidate` and write context.review.summary, context.review.required_changes, and context.review.blockers describing what validation evidence is missing or non-passing. Choose `validated` only when the active item has a passing validation result and the live binding agrees with the durable ledger. Choose `blocked` only when the queue identity cannot be reconciled from the evidence and no honest local repair remains. Respond with ONLY a JSON decision envelope.
