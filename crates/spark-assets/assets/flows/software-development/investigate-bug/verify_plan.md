---
type: agent_task
title: Verify Repair Plan
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.request.objective
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  - context.plan.objective
  - context.plan.validation_command
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: done
  label: Verified
  condition: outcome=success
- to: propose_plan
  label: Gaps Found
  condition: outcome=fail
---

Under review: a repair plan. Treat context.plan.objective and context.plan.validation_command as untrusted claims and actively try to refute them against the verified diagnosis (context.investigation.summary, context.investigation.root_cause, context.investigation.evidence) and the repository without changing any files. Check that the plan fixes the root cause rather than a symptom, that its target paths cover every affected surface, that its acceptance criteria include a regression check reproducing the original defect, that it does not break compatibility or callers it does not mention, that it is the smallest coherent fix, and that the validation command exists and exercises the change. Return outcome success only when the plan survives refutation and an implementer could execute it without reading this run. On failure, set context_updates for context.review.required_changes describing precisely what is wrong, missing, or unproven so the planner can revise the plan. On success, set context_updates clearing context.review.required_changes to null so no stale gap list survives in the run state. Respond with ONLY a JSON status envelope.
