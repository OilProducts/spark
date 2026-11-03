---
type: agent_task
title: Verify Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - context.findings.list
  - context.findings.scope
  writes_context:
  - context.findings.confirmed
  - context.findings.rejected
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: report
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: verify_findings
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a code review. Try to refute every finding in context.findings.list against the repository without modifying any files: re-derive the failure scenario, check whether guards or callers the reviewer missed make it unreachable, and confirm the cited code actually behaves as claimed. Drop findings you refute and findings you cannot verify. Do not add new findings. Choose `done` when verification is complete, with context_updates setting context.findings.confirmed (the surviving findings, most severe first, each with its verified failure scenario and final evidence grade) and context.findings.rejected (refuted findings with the refutation, so the reviewer's misses are auditable). Respond with ONLY a JSON decision envelope.
