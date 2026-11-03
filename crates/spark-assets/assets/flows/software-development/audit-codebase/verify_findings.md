---
type: agent_task
title: Verify Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - parallel.results
  writes_context:
  - context.findings.confirmed
  - context.findings.rejected
  - context.findings.coverage
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

Under review: a codebase audit. parallel.results holds one entry per lens branch (lens_correctness, lens_minimality, lens_safety, lens_test_gaps) in declared order; a `completed` entry's `notes` holds that lens's complete findings (the notes of its `done` decision). A lens whose status is not `completed` (`error`, `terminal_result` or `canceled`) audited nothing: never count it as coverage or as a clean result. Deduplicate findings that describe one underlying defect, then try to refute each against the repository without changing any files: re-derive the failure scenario, check for guards or callers the lens missed, and confirm the cited code behaves as claimed. Drop findings you refute and findings you cannot verify. Do not add new findings. Choose `done` when verification is complete, with context_updates setting context.findings.confirmed (surviving findings, most severe first, each with location, verified failure scenario, and final evidence grade), context.findings.rejected (refuted findings with the refutation, so misses stay auditable), and context.findings.coverage (each lens with its status, and for each lens that did not complete, its recorded `error` and the statement that its surface was not audited). Choose `blocked` when no lens completed or you cannot perform the verification at all. Respond with ONLY a JSON decision envelope.
