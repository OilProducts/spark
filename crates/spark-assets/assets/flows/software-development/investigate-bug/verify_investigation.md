---
type: agent_task
title: Verify Investigation
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.request.objective
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: investigate
  label: Gaps Found
  condition: outcome=fail
- to: propose_plan
  label: Verified
  condition: outcome=success
---

Under review: a bug investigation. Treat context.investigation.summary, context.investigation.root_cause, and context.investigation.evidence as untrusted claims and actively try to refute them against the repository without changing any files. Re-run or independently reproduce the cited evidence where practical; check that the claimed root cause is on the real execution path of the reported defect and not merely nearby suspicious code; look for alternative causes the investigation did not rule out. Grade the evidence: strong (deterministic reproduction, direct inspection), medium (clear code-path reading), weak (assertion, absence of failure). Return outcome success only when the diagnosis survives refutation with strong or sufficient medium evidence for the root cause. On failure, set context_updates for context.review.required_changes describing precisely what is unproven, contradicted, or unexamined so the investigator can close the gaps. On success, set context_updates clearing context.review.required_changes to null so no stale gap list survives in the run state. Respond with ONLY a JSON status envelope.
