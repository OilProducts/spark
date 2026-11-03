---
type: agent_task
title: final_review
contracts:
  reads_context:
  - parallel.results
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: exit
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: final_review
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Finalize and summarize outputs. parallel.results holds one entry per review branch (branch_docs, branch_tests) in declared order; read each entry's `status` and, for a completed branch, its review from that entry's `notes` and `context`. Name every branch whose status is not `completed` (`error`, `terminal_result` or `canceled`) with its recorded `error`, and never present a missing or failed review as approval of what it covered. Respond with ONLY a JSON decision envelope.
