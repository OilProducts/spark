---
type: agent_task
title: Diagnose Run
role: author
runtime:
  session: diagnostician
  max_entries: 4
contracts:
  reads_context:
  - context.retro.run_dir
  - context.review.required_changes
  writes_context:
  - context.review.contested
  - context.retro.diagnosis
  - context.retro.evidence
  response: status_envelope
routes:
- to: verify_diagnosis
  condition: outcome=success
---

Diagnose the run recorded at context.retro.run_dir without changing any files. Read its record, events.jsonl, checkpoint, and per-node logs (prompts, status envelopes, tool output) to reconstruct what the run was asked to do, what each node actually did, and where the outcome diverged from intent. Classify the failure honestly: prompt gap (the node was instructed badly), contract violation (the node answered outside its envelope or context contract), flow design (the graph routed or looped wrongly), environment or validation flakiness, or a runtime defect — and distinguish the triggering node from downstream casualties. If context.review.required_changes is present, it lists gaps a verifier found in your previous diagnosis; resolve each one as your role describes. Cite evidence by artifact file and content, not by summary. Respond with ONLY a JSON status envelope whose context_updates set context.retro.diagnosis (what went wrong and why, with the failure class), and context.retro.evidence (the artifact citations supporting it).
