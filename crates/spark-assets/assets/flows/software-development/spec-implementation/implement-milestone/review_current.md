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
  - internal.project_work_dir
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: mark_current_blocked
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  fix:
    to: implement_current
    description: The completed review or validation assessment found concrete defects requiring another implementation pass on the same item; record actionable findings.
  rewrite:
    to: rewrite_current
    description: The current item or milestone decomposition is unsuitable; write the required replacement-work ledger before rewriting it.
  unfinished:
    to: review_current
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  validate:
    to: prepare_validation
    description: The item review found the implementation complete enough for the downstream repository validation gate; this does not assert validation has passed.
---

Review the current item against context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, <internal.project_work_dir>/spec-implementation/current/spec/source.md, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, and the milestone state. In active-workspace shorthand, the architecture file is <internal.project_work_dir>/spec-implementation/current/spec/architecture.md and the decision ledger is <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. Treat <internal.project_work_dir>/spec-implementation/current/spec/source.md as product behavior, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md as the binding implementation contract, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json as the approved resolution set for the item's bound decisions. Judge the touched surfaces as deliverable, not merely locally functional. When applicable, confirm the repository can be run, imported, and validated from committed repo metadata, manifests, bootstrap files, docs, and exports; that public use does not depend on test-only bootstrap or path rewriting; that tests are behavioral rather than brittle source-text checks; that tracked local, generated, cache, or IDE artifacts were not added as deliverables; that the touched area did not introduce obvious duplicate validators or mixed-responsibility growth; and that docs, exports, manifests, and ignore rules remain coherent. Treat helper-only validation of a documented public surface as incomplete. Treat wrapper-only or duplicate delivery layers as incomplete unless a real external compatibility contract requires them. Fail if code or tests satisfy a weaker contract than the item's bound decisions, or if a material spec-versus-architecture conflict was chosen silently instead of being resolved in <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. This is the pre-validation deliverability review; do not require or assess validation-result.json here because the unchanged repository-wide validation and completion gate follow a successful review. If the current item is complete enough for the downstream repository validation gate, choose `validate`. If more implementation work is needed on the same item, choose `fix` and write context.review.summary, context.review.required_changes, and context.review.blockers. If the item is too broad, wrongly scoped, or exposed prerequisite repair work that should happen before it can complete cleanly, write milestone_dir/rewrite-items.json as a JSON array of replacement items using the same item schema and choose `rewrite`. Use `rewrite` for systemic repository-integrity or architecture fixes that can still be resolved within this milestone. Ambiguity is never a reason to block an item: choose the most defensible interpretation consistent with the spec, record it as a new decision in <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, and request any resulting changes through `fix`. Write <internal.project_work_dir>/spec-implementation/current/block-reason.md and choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Do not mutate the queue state directly in this node. Respond with ONLY a JSON decision envelope.
