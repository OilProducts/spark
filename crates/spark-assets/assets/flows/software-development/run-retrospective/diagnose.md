---
type: agent_task
title: Diagnose Run
role: author
runtime:
  session: diagnostician
  max_visits: 4
contracts:
  reads_context:
  - context.retro.run_dir
  - context.review.required_changes
  writes_context:
  - context.review.contested
  - context.retro.diagnosis
  - context.retro.evidence
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready_for_review:
    to: verify_diagnosis
    description: The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats.
  unfinished:
    to: diagnose
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Diagnose the run recorded at context.retro.run_dir without changing any files. Read its record, events.jsonl, checkpoint, and per-node logs (prompts, response envelopes, tool output) to reconstruct what the run was asked to do, what each node actually did, and where the outcome diverged from intent. Classify the failure honestly: prompt gap (the node was instructed badly), contract violation (the node answered outside its envelope or context contract), flow design (the graph routed or looped wrongly), environment or validation flakiness, or a runtime defect — and distinguish the triggering node from downstream casualties. If context.review.required_changes is present, it lists gaps a verifier found in your previous diagnosis; resolve each one as your role describes. Cite evidence by artifact file and content, not by summary. Choose `ready_for_review` when the diagnosis is complete enough for independent verification, recording any caveats honestly, with context_updates setting context.retro.diagnosis (what went wrong and why, with the failure class), and context.retro.evidence (the artifact citations supporting it). Respond with ONLY a JSON decision envelope.
