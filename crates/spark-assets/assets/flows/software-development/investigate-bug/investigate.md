---
type: agent_task
title: Investigate
runtime:
  session: investigator
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.review.required_changes
  writes_context:
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  response: status_envelope
routes:
- to: verify_investigation
  condition: outcome=success
---

Investigate the defect described by context.request.objective in the current checkout without changing any files. Reproduce or characterize the failure through real interfaces (tests, CLI, API, observable state), trace the failing execution path, and identify the root cause and affected surfaces. If context.review.required_changes is present, it lists gaps a verifier found in your previous investigation; close them. Prefer deterministic evidence: commands you ran, files and line ranges you inspected, observed versus expected behavior. Do not propose or implement fixes yet. Respond with ONLY a JSON status envelope whose context_updates set context.investigation.summary (the diagnosis), context.investigation.root_cause (the specific defective code path), and context.investigation.evidence (the concrete evidence trail: commands, outputs, file references).
