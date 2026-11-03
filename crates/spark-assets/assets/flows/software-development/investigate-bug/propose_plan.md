---
type: agent_task
title: Propose Repair Plan
role: author
runtime:
  session: planner
  max_visits: 4
contracts:
  reads_context:
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  - context.request.validation_command
  - context.review.required_changes
  writes_context:
  - context.review.contested
  - context.plan.objective
  - context.plan.validation_command
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready_for_review:
    to: verify_plan
    description: The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats.
  unfinished:
    to: propose_plan
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Turn the verified diagnosis (context.investigation.summary, context.investigation.root_cause, context.investigation.evidence) into one decision-complete repair plan without changing any files. The plan must state: the bounded objective of the fix, the target paths, explicit acceptance criteria including a regression check that reproduces the original defect, compatibility constraints, and the truthful validation command (use context.request.validation_command when provided, otherwise discover it from the repository). Prefer the smallest coherent fix over redesign. If context.review.required_changes is present, it lists gaps a verifier found in your previous plan; resolve each one as your role describes. Choose `ready_for_review` when the plan is complete enough for independent verification, with context_updates setting context.plan.objective (a self-contained task statement an implementer can execute without reading this run) and context.plan.validation_command. Respond with ONLY a JSON decision envelope.
