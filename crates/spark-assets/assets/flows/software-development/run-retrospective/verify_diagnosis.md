---
type: agent_task
title: Verify Diagnosis
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.retro.run_dir
  - context.retro.diagnosis
  - context.retro.evidence
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: diagnose
  label: Gaps Found
  condition: outcome=fail
- to: recommend
  label: Verified
  condition: outcome=success
---

Under review: a run diagnosis. Treat context.retro.diagnosis and context.retro.evidence as untrusted claims and try to refute them against the artifacts under context.retro.run_dir without changing any files: re-read the cited events and logs, check the claimed triggering node actually precedes the casualties, and look for an earlier cause the diagnosis skipped. Return outcome success only when the diagnosis survives with evidence anchored in the artifacts. On failure, set context_updates for context.review.required_changes describing precisely what is unproven, contradicted, or unexamined. On success, set context_updates clearing context.review.required_changes to null so no stale gap list survives in the run state. Respond with ONLY a JSON status envelope.
