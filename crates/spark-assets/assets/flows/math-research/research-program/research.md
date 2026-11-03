---
type: agent_task
title: Do Mathematics
contracts:
  reads_context:
  - context.request.problem
  - context.request.focus
  - context.action.repair
  writes_context:
  - context.action.id
  - context.action.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: verify
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: research
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Advance the mathematical problem by one bounded action capable of making substantive progress. Read problem.md and state.md first, consulting relevant history/ records rather than reading all history blindly. If problem.md is empty or still a template, initialize it from context.request.problem and context.request.focus without silently changing the problem. If state.md is empty or still a template, initialize its current claim state from the workspace evidence and known results, labeling uncertainty honestly. Judge progress by how an action can change the current research frontier, not merely by whether its method, artifacts, or numerical scope differ from previous work. An action may be substantive if it establishes or refutes a structural claim, removes a genuine obstacle to resolution, tests a qualitatively different mechanism, crosses a mathematically meaningful threshold, closes a necessary finite case, supplies an induction base, distinguishes competing conjectures, or enables a concrete proof step. Changing a finite bound may be substantive when its mathematical consequence is stated; it is not substantive solely because the bound is larger, smaller, or independently recomputed. Choose the feasible substantive action most likely to change the state: it may combine literature work, examples, computation, counterexample search, proof development, reduction, statement clarification, or formalization. Before working, choose the next unused sequential action ID A0001, A0002, ...; write it to .math-research/current-action; create evidence/<ID>/; and write evidence/<ID>/plan.md stating the target claim, current uncertainty, possible outcomes, resource bound, intended frontier change, and why that change would be substantive relative to prior actions and failed attacks. Then do the mathematics and preserve all outputs under evidence/<ID> (existing relevant workspace artifacts may be copied or referenced exactly). Write report.md and claim.json there. claim.json must identify the exact proposed claims, their type, the problem-statement version, supporting artifacts, relevant tool versions, and whether each claim is machine-checkable. For any machine-checkable claim write verify.sh that independently checks it and exits nonzero on mismatch; do not create resolution.json yourself. Do not update state.md or history/ yet—the independent assessor and recorder own acceptance. A completed bounded action is `done` even when it only makes partial mathematical progress or identifies an obstruction; choose `unfinished` only when this bounded action's own required outputs are still incomplete, never merely because the problem remains open. Set context_updates for context.action.id and context.action.summary. Respond with ONLY a JSON decision envelope.
