---
type: agent_task
title: Route Request
contracts:
  reads_context:
  - context.request.artifact_path
  response: decision_envelope
execution:
  reasoning_effort: low
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  existing:
    to: draft
    description: context.request.artifact_path names an existing design, so no approaches are drafted.
  new:
    to: explore
    description: No existing design was given, so competing approaches are drafted.
  unfinished:
    to: route_request
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Choose `existing` when context.request.artifact_path names a design, and `new` when it is missing or empty. Do not read or change any files. Respond with ONLY a JSON decision envelope.
