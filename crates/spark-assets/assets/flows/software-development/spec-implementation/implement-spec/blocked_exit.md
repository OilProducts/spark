---
type: agent_task
title: Blocked Exit
runtime: {}
contracts:
  response: decision_envelope
retry:
  max_retries: 1
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: failed
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: blocked_exit
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Inspect .spark/state/spec-implementation/current/state.json, .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/milestones.json, and any current milestone artifacts. In notes, write a short summary of what remains blocked and why the program could not complete automatically, citing the originating blocker evidence. Choose `done` once that summary is complete. Respond with ONLY a JSON decision envelope.
