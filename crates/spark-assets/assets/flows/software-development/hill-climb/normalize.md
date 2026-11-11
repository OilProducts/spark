---
type: agent_task
title: Normalize
contracts:
  reads_context:
  - context.request.objective
  - context.request.rubric
  - context.request.target_score
  - context.request.max_rounds
  - context.request.evidence_command
  - context.request.references
  - context.request.base_ref
  writes_context:
  - context.climb.objective
  - context.climb.rubric
  - context.climb.target_score
  - context.climb.max_rounds
  - context.climb.evidence_command
  - context.climb.references
  - context.climb.base_ref
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: The request cannot be turned into a climb, such as a missing objective or a target above 10; record why in notes.
  done:
    to: prepare_workspace
    description: Every context.climb value is set.
---

Turn the launch request into the climb's settings without changing any files. Set context.climb.objective to the objective. Set context.climb.rubric to the request's rubric, or, when it has none, to three to six distinct categories that together cover what the objective asks to improve. Set context.climb.target_score to the requested target, or 9; it must be above 0 and at most 10. Set context.climb.max_rounds to the requested whole number of rounds, or 8; it must be from 1 to 20. Set context.climb.evidence_command to the requested command, or an empty string. Set context.climb.references to the requested references, or an empty list. Set context.climb.base_ref to the requested ref, or HEAD. Respond with ONLY a JSON decision envelope.
