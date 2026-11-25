---
type: agent_task
title: Verify Review
role: adversarial-reviewer
contracts:
  reads_context:
  - context.request.mission_id
  - context.retro.state_db
  - context.retro.runs_dir
  - context.retro.assessment
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
    to: review_mission
    description: The completed verification found unproven, contradicted or missing claims; record each gap and the evidence needed to resolve it.
  unfinished:
    to: verify_review
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  verified:
    to: recommend
    description: The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed.
---

Under review: a mission retrospective for `context.request.mission_id`. Try to refute `context.retro.assessment` and `context.retro.evidence` against the mission record and conversation in `context.retro.state_db` (read-only), the child runs under `context.retro.runs_dir`, and the repository without changing any files. Re-check cited run counts, statuses, and commit hashes; verify that claimed root causes actually precede downstream casualties; test whether any item claimed as covered was silently missed or weakened; and look for major failures, churn, or playbook/flow defects the review overlooked.

If `context.review.contested` is present, it lists earlier findings the author contests, each with its evidence; re-check each one. Apply a materiality bar when deciding between `verified` and `gaps`: choose `gaps` only for material factual errors, unsupported or contradicted conclusions, misattributed root causes, or major omitted failures or scope gaps that would change the takeaway or recommendations. Do not bounce for minor timing subdivisions, secondary wording polish, or low-impact caveats once the core chronology, counts, and findings hold up — record minor nuances in `notes` and choose `verified`, clearing `context.review.required_changes` and `context.review.contested` to null so no stale gap list survives. When choosing `gaps`, set `context.review.required_changes` describing precisely each material gap and what evidence resolves it. Choose `blocked` only when you cannot perform the verification at all. Respond with ONLY a JSON decision envelope.
