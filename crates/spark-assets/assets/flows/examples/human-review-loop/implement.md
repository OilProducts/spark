---
type: agent_task
title: Prepare Change
contracts:
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: review_gate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: implement
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Inspect the repository, prepare the proposed change for the stated goal, and summarize what is ready for review. Respond with ONLY a JSON decision envelope.
