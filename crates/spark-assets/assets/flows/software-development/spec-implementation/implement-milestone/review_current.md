---
type: agent_task
title: Review Current Item
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.item.decision_ids
  - context.item.acceptance_criteria
  - context.item.constraints
  - context.item.target_paths
  - context.item.attempts
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: status_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: implement_current
  label: Fix
  condition: outcome=fail && preferred_label=Fix
  extensions:
    loop_restart: false
- to: implement_current
  label: Fix Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: mark_current_blocked
  label: Block
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: prepare_validation
  label: Validate
  condition: outcome=success
  extensions:
    loop_restart: false
- to: rewrite_current
  label: Rewrite
  condition: outcome=fail && preferred_label=Rewrite
  extensions:
    loop_restart: false
- to: mark_current_blocked
  label: Block Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
- to: rewrite_current
  label: Rewrite Partial
  condition: outcome=partial_success && preferred_label=Rewrite
  weight: 1
  extensions:
    loop_restart: false
---

Review the current item against context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/architecture.md, .spark/spec-implementation/current/spec/contract-decisions.json, and the milestone state. In active-workspace shorthand, the architecture file is .spark/spec-implementation/current/spec/architecture.md and the decision ledger is .spark/spec-implementation/current/spec/contract-decisions.json. Treat .spark/spec-implementation/current/spec/source.md as product behavior, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved resolution set for the item's bound decisions. Judge the touched surfaces as deliverable, not merely locally functional. When applicable, confirm the repository can be run, imported, and validated from committed repo metadata, manifests, bootstrap files, docs, and exports; that public use does not depend on test-only bootstrap or path rewriting; that tests are behavioral rather than brittle source-text checks; that tracked local, generated, cache, or IDE artifacts were not added as deliverables; that the touched area did not introduce obvious duplicate validators or mixed-responsibility growth; and that docs, exports, manifests, and ignore rules remain coherent. Treat helper-only validation of a documented public surface as incomplete. Treat wrapper-only or duplicate delivery layers as incomplete unless a real external compatibility contract requires them. Fail if code or tests satisfy a weaker contract than the item's bound decisions, or if a material spec-versus-architecture conflict was chosen silently instead of being resolved in .spark/spec-implementation/current/spec/contract-decisions.json. This is the pre-validation deliverability review; do not require or assess validation-result.json here because the unchanged repository-wide validation and completion gate follow a successful review. Respond with ONLY a JSON status envelope. If the current item is done, return outcome success. If more implementation work is needed on the same item, return outcome fail with preferred_label Fix and write context.review.summary, context.review.required_changes, and context.review.blockers. If the item is too broad, wrongly scoped, or exposed prerequisite repair work that should happen before it can complete cleanly, write milestone_dir/rewrite-items.json as a JSON array of replacement items using the same item schema and return outcome fail with preferred_label Rewrite. Use Rewrite for systemic repository-integrity or architecture fixes that can still be resolved within this milestone. Ambiguity is never a reason to block an item: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, and request any resulting changes through Fix. Write .spark/spec-implementation/current/block-reason.md and return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Do not mutate the queue state directly in this node.
