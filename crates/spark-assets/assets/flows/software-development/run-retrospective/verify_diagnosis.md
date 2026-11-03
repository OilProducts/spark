---
type: agent_task
title: Verify Diagnosis
role: adversarial-reviewer
contracts:
  reads_context:
  - context.retro.run_dir
  - context.retro.diagnosis
  - context.retro.evidence
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
    to: diagnose
    description: The completed verification found unproven, contradicted or missing claims; record each gap and the evidence needed to resolve it.
  unfinished:
    to: verify_diagnosis
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  verified:
    to: recommend
    description: The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed.
---

Under review: a run diagnosis. Try to refute context.retro.diagnosis and context.retro.evidence against the artifacts under context.retro.run_dir without changing any files: re-read the cited events and logs, check the claimed triggering node actually precedes the casualties, and look for an earlier cause the diagnosis skipped. Choose `verified` only when the diagnosis survives with evidence anchored in the artifacts. Otherwise choose `gaps` and set context_updates for context.review.required_changes describing precisely what is unproven, contradicted, or unexamined. If context.review.contested is present, it lists earlier findings the author contests, each with its evidence; re-check each one. With `verified`, set context_updates clearing context.review.required_changes and context.review.contested to null so no stale gap list survives in the run state. Choose `blocked` only when you cannot perform the verification at all. Respond with ONLY a JSON decision envelope.
