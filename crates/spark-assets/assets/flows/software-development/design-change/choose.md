---
type: agent_task
title: Choose Approach
role: adversarial-reviewer
contracts:
  reads_context:
  - parallel.results
  - context.request.objective
  - context.request.constraints
  writes_context:
  - context.approach.chosen
  - context.approach.rejected
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: draft
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: choose
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

parallel.results holds one entry per draft in declared order. For a `completed` entry, context.approach.draft in that entry's context holds the draft, and the entry's id names its aim and model. A draft whose status is not `completed`, or that left context.approach.draft unset, is not a candidate.

Choose the one draft that best serves context.request.objective within context.request.constraints. Judge how completely each removes the root problem and how simple the resulting system would be: the fewest concepts, the least code, and nothing the request does not need. Do not prefer a draft for being the smaller change; the cost of the change counts only as a risk to weigh. Try to refute each draft's claims against the current code before relying on them. Pick one draft without merging drafts.

Do not change any files. Choose `done` with context_updates setting context.approach.chosen to the chosen draft's full text, unchanged, and context.approach.rejected to each other approach with the concrete reason it lost, treating drafts that propose the same approach as one. Also record there any measurements, facts or leads from rejected drafts that the design should use, saying which you verified. Choose `blocked` when no draft is a candidate. Respond with ONLY a JSON decision envelope.
