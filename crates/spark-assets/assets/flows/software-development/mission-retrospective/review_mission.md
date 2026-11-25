---
type: agent_task
title: Review Mission
role: author
runtime:
  session: reviewer
  max_visits: 4
contracts:
  reads_context:
  - context.request.mission_id
  - context.request.focus
  - context.retro.state_db
  - context.retro.project_id
  - context.retro.runs_dir
  - context.review.required_changes
  writes_context:
  - context.review.contested
  - context.retro.assessment
  - context.retro.evidence
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready_for_review:
    to: verify_review
    description: The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats.
  unfinished:
    to: review_mission
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Review how mission `context.request.mission_id` went without changing any files. Read its mission record, conversation turns, segments, and inbox events read-only from `context.retro.state_db`, and inspect its child runs (run records, `events.jsonl`, per-node logs, review reports, and merge outcomes) under `context.retro.runs_dir`, along with any source reports, playbooks, or merged commits the mission references. Weight `context.request.focus` when present.

Assess the mission honestly across four areas:
1. **Goal and scope coverage:** Did the mission deliver everything its description and source artifacts asked for? Flag any requirement or finding silently dropped, deferred without justification, or merged without independent verification.
2. **Orchestration and efficiency:** Evaluate batching, review/merge sequencing, budget usage, and failure recovery. Quantify churn and wasted work (failed or canceled runs, visit-limit hits, repeated review loops, stale-branch merge conflicts, or duplicate fixes).
3. **Systemic friction and defects:** Classify concrete problems honestly — playbook or mission-decision mistakes, flow/prompt gaps, validation or environment flakiness, or runtime/CLI defects — distinguishing root causes from downstream casualties.
4. **What worked well:** Briefly note practices or checks that caught real defects or prevented bad merges.

If `context.review.required_changes` is present, it lists gaps an adversarial verifier found in your previous assessment; resolve each one as your role describes. Cite evidence directly by mission turn or segment ID, run ID, artifact file path, commit hash, and exact counts — never rely on summaries alone. Choose `ready_for_review` when the assessment is complete enough for independent verification, recording any caveats honestly, with `context_updates` setting `context.retro.assessment` (structured findings covering all four areas) and `context.retro.evidence` (the concrete citations supporting each claim). Respond with ONLY a JSON decision envelope.
