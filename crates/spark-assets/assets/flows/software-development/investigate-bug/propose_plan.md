---
type: agent_task
title: Propose Repair Plan
runtime:
  session: planner
contracts:
  reads_context:
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  - context.request.validation_command
  - context.review.required_changes
  writes_context:
  - context.plan.objective
  - context.plan.validation_command
  response: status_envelope
routes:
- to: verify_plan
  condition: outcome=success
---

Turn the verified diagnosis (context.investigation.summary, context.investigation.root_cause, context.investigation.evidence) into one decision-complete repair plan without changing any files. The plan must state: the bounded objective of the fix, the target paths, explicit acceptance criteria including a regression check that reproduces the original defect, compatibility constraints, and the truthful validation command (use context.request.validation_command when provided, otherwise discover it from the repository). Prefer the smallest coherent fix over redesign. If context.review.required_changes is present, it lists gaps a verifier found in your previous plan; close them. Respond with ONLY a JSON status envelope whose context_updates set context.plan.objective (a self-contained task statement an implementer can execute without reading this run) and context.plan.validation_command.
