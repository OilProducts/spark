---
type: agent_task
title: Verify Repair Plan
role: adversarial-reviewer
contracts:
  reads_context:
  - context.request.objective
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  - context.plan.objective
  - context.plan.validation_command
  - context.review.contested
  writes_context:
  - context.review.required_changes
  - context.review.contested
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  gaps:
    to: propose_plan
    description: The completed verification found unproven, contradicted or missing claims; record each gap and the evidence needed to resolve it.
  unfinished:
    to: verify_plan
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  verified:
    to: done
    description: The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed.
---

Under review: a repair plan. Try to refute context.plan.objective and context.plan.validation_command against the verified diagnosis (context.investigation.summary, context.investigation.root_cause, context.investigation.evidence) and the repository without changing any files. Check that the plan fixes the root cause rather than a symptom, that its target paths cover every affected surface, that its acceptance criteria include a regression check reproducing the original defect, that it does not break compatibility or callers it does not mention, that it is the smallest coherent fix, and that the validation command exists and exercises the change. Choose `verified` only when the plan survives refutation and an implementer could execute it without reading this run. Otherwise choose `gaps` and set context_updates for context.review.required_changes describing precisely what is wrong, missing, or unproven so the planner can revise the plan. If context.review.contested is present, it lists earlier findings the author contests, each with its evidence; re-check each one. With `verified`, set context_updates clearing context.review.required_changes and context.review.contested to null so no stale gap list survives in the run state. Choose `blocked` only when you cannot perform the verification at all. Respond with ONLY a JSON decision envelope.
