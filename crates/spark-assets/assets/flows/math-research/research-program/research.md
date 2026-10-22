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
  response: status_envelope
routes:
- to: verify
  condition: outcome=success
- to: verify
  condition: outcome=partial_success
---

Advance the mathematical problem by one bounded action capable of making substantive progress. Read problem.md and state.md first, consulting relevant history/ records rather than reading all history blindly. If problem.md is empty or still a template, initialize it from context.request.problem and context.request.focus without silently changing the problem. If state.md is empty or still a template, initialize its current claim state from the workspace evidence and known results, labeling uncertainty honestly. Judge progress by how an action can change the current research frontier, not merely by whether its method, artifacts, or numerical scope differ from previous work. An action may be substantive if it establishes or refutes a structural claim, removes a genuine obstacle to resolution, tests a qualitatively different mechanism, crosses a mathematically meaningful threshold, closes a necessary finite case, supplies an induction base, distinguishes competing conjectures, or enables a concrete proof step. Changing a finite bound may be substantive when its mathematical consequence is stated; it is not substantive solely because the bound is larger, smaller, or independently recomputed. Choose the feasible substantive action most likely to change the state: it may combine literature work, examples, computation, counterexample search, proof development, reduction, statement clarification, or formalization. Before working, choose the next unused sequential action ID A0001, A0002, ...; write it to .math-research/current-action; create evidence/<ID>/; and write evidence/<ID>/plan.md stating the target claim, current uncertainty, possible outcomes, resource bound, intended frontier change, and why that change would be substantive relative to prior actions and failed attacks. Then do the mathematics and preserve all outputs under evidence/<ID> (existing relevant workspace artifacts may be copied or referenced exactly). Write report.md and claim.json there. claim.json must identify the exact proposed claims, their type, the problem-statement version, supporting artifacts, relevant tool versions, and whether each claim is machine-checkable. For any machine-checkable claim write verify.sh that independently checks it and exits nonzero on mismatch; do not create resolution.json yourself. Do not update state.md or history/ yet—the independent assessor and recorder own acceptance. A completed bounded action is outcome success even when it only makes partial mathematical progress or identifies an obstruction. Respond with ONLY a JSON status envelope whose outcome is success and whose context_updates set context.action.id and context.action.summary.
