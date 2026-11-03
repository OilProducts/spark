---
type: agent_task
title: Verify Investigation
role: adversarial-reviewer
contracts:
  reads_context:
  - context.request.objective
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
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
    to: investigate
    description: The completed verification found unproven, contradicted or missing claims; record each gap and the evidence needed to resolve it.
  unfinished:
    to: verify_investigation
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  verified:
    to: propose_plan
    description: The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed.
---

Under review: a bug investigation. Try to refute context.investigation.summary, context.investigation.root_cause, and context.investigation.evidence against the repository without changing any files. Re-run or independently reproduce the cited evidence where practical; check that the claimed root cause is on the real execution path of the reported defect and not merely nearby suspicious code; look for alternative causes the investigation did not rule out. Refute a diagnosis that stops at a missing piece or difference in a separate implementation of something the codebase already does elsewhere: the root cause is the duplication, and the diagnosis must name it. Choose `verified` only when the diagnosis survives refutation and the root cause is established. Otherwise choose `gaps` and set context_updates for context.review.required_changes describing precisely what is unproven, contradicted, or unexamined so the investigator can close the gaps. If context.review.contested is present, it lists earlier findings the author contests, each with its evidence; re-check each one. With `verified`, set context_updates clearing context.review.required_changes and context.review.contested to null so no stale gap list survives in the run state. Choose `blocked` only when you cannot perform the verification at all. Respond with ONLY a JSON decision envelope.
