---
type: agent_task
title: Synthesize
contracts:
  reads_context:
  - context.survey.summary
  - context.experiments.summary
  - context.review.required_changes
  writes_context:
  - context.findings.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: review_findings
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: synthesize
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Synthesize the verified experimental results into findings.md: what was computed, what it suggests, and what it rules out. Label every claim per AGENTS.md — new proof, formalization of a known result, located in literature, or computational evidence — and state the exact scope of each computation (parameters covered, cases exhausted, certificates checked). Conjectures must be stated as conjectures with the evidence that motivates them and the smallest open case. If context.review.required_changes is present, close those findings or labeling gaps. Then append a session entry to DOSSIER.md per its structure and update its status board: outcome, established results, failed attacks with what they taught, verified data locations, and open subproblems; dossier entries are append-only. Propose the natural next session in a closing section of findings.md. Set context_updates for context.findings.summary, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
