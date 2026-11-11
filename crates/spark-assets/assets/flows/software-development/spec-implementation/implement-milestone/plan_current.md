---
type: agent_task
title: Plan Current Item
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
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: implement_current
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: plan_current
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/architecture.md and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. In active-workspace shorthand, the decision ledger is <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. Inspect files needed from the current repository and produce a concrete implementation plan for the current milestone item defined by context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, and context.item.target_paths. Treat <internal.project_work_dir>/spec-implementation/current/spec/source.md as product behavior, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md as the binding implementation contract, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json as the approved resolution set for the item's bound decisions. Convert the item's decision_ids into explicit implementation and validation goals. Include the proof of done for this item: what evidence review should expect, what validation scope is appropriate, and which checks are required before the item can be considered complete. Default proof of done to the strongest deterministic repo-wide validation gate. If this item changes a documented public surface, include at least one check through that surface and explain how the bound decisions will be satisfied. When relevant, cover public bootstrap and installability, import and export wiring, validation truthfulness, docs or manifest updates, ignore rules, and touched-surface cohesion so the item can finish deliverably rather than only locally. Stay within the current cwd (the repository root) and <internal.project_work_dir>/spec-implementation/. Do not inspect paths outside it. If review feedback exists in context.review.*, use it to refine the plan instead of repeating the item description. If the item appears to require prerequisite repair or broader repository-integrity work before it can complete cleanly, plan toward a review `rewrite` rather than a brittle local workaround or a weaker contract than the bound decisions. Choose `done` once the concrete plan, including its proof of done, is complete. Respond with ONLY a JSON decision envelope.
