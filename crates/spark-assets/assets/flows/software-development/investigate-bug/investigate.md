---
type: agent_task
title: Investigate
role: author
runtime:
  session: investigator
  max_visits: 4
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.review.required_changes
  writes_context:
  - context.review.contested
  - context.investigation.summary
  - context.investigation.root_cause
  - context.investigation.evidence
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready_for_review:
    to: verify_investigation
    description: The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats.
  unfinished:
    to: investigate
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Investigate the defect described by context.request.objective in the current checkout without changing any files. Reproduce or characterize the failure through real interfaces (tests, CLI, API, observable state), trace the failing execution path, and identify the root cause and affected surfaces. If the defect is a missing piece or a difference in a separate implementation of something the codebase already does elsewhere, the root cause is the duplication: name both implementations in the diagnosis, not just the missing piece in the copy. If context.review.required_changes is present, it lists gaps a verifier found in your previous investigation; resolve each one as your role describes. Prefer deterministic evidence: commands you ran, files and line ranges you inspected, observed versus expected behavior. Do not propose or implement fixes yet. Choose `ready_for_review` when the investigation is complete enough for independent verification, recording any caveats honestly, with context_updates setting context.investigation.summary (the diagnosis), context.investigation.root_cause (the specific defective code path), and context.investigation.evidence (the concrete evidence trail: commands, outputs, file references). Respond with ONLY a JSON decision envelope.
