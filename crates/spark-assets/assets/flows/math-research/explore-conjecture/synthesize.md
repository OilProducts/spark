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
  response: status_envelope
routes:
- to: review_findings
---

Synthesize the verified experimental results into findings.md: what was computed, what it suggests, and what it rules out. Label every claim per AGENTS.md — new proof, formalization of a known result, located in literature, or computational evidence — and state the exact scope of each computation (parameters covered, cases exhausted, certificates checked). Conjectures must be stated as conjectures with the evidence that motivates them and the smallest open case. If context.review.required_changes is present, close those findings or labeling gaps. Then append a session entry to DOSSIER.md per its structure and update its status board: outcome, established results, failed attacks with what they taught, verified data locations, and open subproblems; dossier entries are append-only. Propose the natural next session in a closing section of findings.md. Respond with ONLY a JSON status envelope whose context_updates set context.findings.summary.
