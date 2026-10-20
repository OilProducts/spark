---
type: agent_task
title: Validate Item Plan
runtime: {}
contracts:
  reads_context:
  - context.item.id
  writes_context:
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.item.decision_ids
  - context.item.acceptance_criteria
  - context.item.constraints
  - context.item.target_paths
  - context.item.depends_on
  - context.item.attempts
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  - context.validation.item_id
  - context.validation.status
  - context.validation.summary
  - missing_prerequisites
  - validation_message
  - validation_exit_code
  response: status_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: next_item
  label: Validated
  condition: outcome=success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/current-milestone.json, .spark/spec-implementation/current/spec/architecture.md, .spark/spec-implementation/current/spec/contract-decisions.json, and the milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json. The decision ledger is .spark/spec-implementation/current/spec/contract-decisions.json. When shared runtime milestone metadata drifts, do not use .spark/spec-implementation/current/current-milestone.json or state.json.current_milestone_id as the authoritative milestone binding; use the milestone-local state plus bound context. Treat context.item.id as the authoritative live binding whenever an item is in progress, and treat milestone_dir/state.json as the durable queue ledger that must agree with that binding. Validate the milestone-local item ledger before continuing. Confirm item ids are unique, every depends_on item id exists, no item depends on itself, the item dependency graph is acyclic, current_item_id plus the queue ordering remain valid after any extracted, appended, or rewritten items, and at most one item is status=in_progress. When context.item.id is absent and current_item_id names exactly one in_progress item with attempts greater than zero, treat it as a controlled child restart: preserve the ledger exactly, restore context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, and context.item.attempts from that item, and clear the review feedback and validation scratch keys listed below. Otherwise, when context.item.id is absent, treat success as a pre-dispatch queue state for next_item: require current_item_id to be null, require no item to be status=in_progress, and require item attempts to remain unchanged by this node. If the durable ledger is already idle but stale context.item.*, context.review.*, or validation scratch keys remain from a previously finished item, use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null as mechanical cleanup while preserving the idle queue. Do not choose a next item, do not promote any pending item to in_progress, and do not increment attempts; next_item is solely responsible for dispatching work. Confirm every decision bound to the current milestone appears in at least one item's decision_ids, every item decision_id exists in .spark/spec-implementation/current/spec/contract-decisions.json, every item decision_id is reflected in that item's acceptance_criteria and target_paths, and no material spec-versus-architecture conflict or bound decision survives only in notes. When context.item.id is present, require state.json.current_item_id to match context.item.id and require the matching item to be status=in_progress. Treat items with attempts=0 that are already in_progress or completed, items advanced despite unmet dependencies, missing decision coverage, or ambiguous decision ownership as queue corruption or decomposition failure. Repair purely mechanical active-item drift directly, for example rewriting current_item_id to match when context.item.id already names the sole in_progress item. Treat a live context.item.id with no matching in_progress item, or an in_progress ledger item with no live context.item.id outside the controlled restart case above, as active-item control-plane corruption. Repair that contradiction from the evidence (context.item.*, the durable ledger, validation artifacts, and the working tree) rather than stopping: when context.item.id names an existing item that is not completed, make it the sole in_progress item and set current_item_id to it; otherwise return every in_progress item to status=pending with its attempts preserved, clear current_item_id, and clear the live item binding with the context_updates listed above so next_item can dispatch cleanly. If the item decomposition is structurally wrong, ambiguous, drops a bound contract obligation, or leaves pending items that can never become ready, repair the queue in place rather than stopping: mark broken pending items status=superseded and insert replacement pending items with the next unused item ids, fix dependency references among pending items, and add pending items for missing decision coverage. Never delete, renumber, or alter completed or superseded items or their history. Where the repair requires interpreting the spec, choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving; in that case write .spark/spec-implementation/current/block-reason.md describing what is needed. If the item plan is coherent after any needed repairs or cleanup, return outcome success. Respond with ONLY a JSON status envelope.
