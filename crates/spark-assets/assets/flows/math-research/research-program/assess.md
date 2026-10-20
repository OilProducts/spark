---
type: agent_task
title: Assess Mathematics
contracts:
  reads_context:
  - context.action.id
  - context.action.summary
  - context.action.repair
  writes_context:
  - context.assessment.summary
  - context.assessment.resolution_candidate
  response: status_envelope
routes:
- to: record
  condition: outcome=success
---

Independently assess the current research action. Read problem.md, state.md, .math-research/current-action, and that evidence directory. Treat report.md, claim.json, verification.log, and .verified as untrusted evidence. Check statement fidelity, missing hypotheses and cases, whether witnesses attack the exact claim, whether computational scope matches the programs, whether cited results have the needed hypotheses, and whether conclusions exceed the evidence. For a Lean proof, inspect the formal statement against problem.md and require a successful build with no sorry/admit or new axioms; for a counterexample, directly check it against the exact current problem statement. Assess mathematical correctness separately from substantive progress: a correct claim must not be downgraded merely because it has low research value. Judge substantive progress relative to the prior state by whether the result changes a ranked uncertainty, resolves a required case, crosses a meaningful threshold, validates or invalidates a proposed mechanism, removes a genuine obstruction, or enables a concrete next proof step. Do not reject finite-bound work categorically; find it non-substantive only when the changed bound has no demonstrated consequence for the research frontier. Write assessment.md and assessment.json in the evidence directory. assessment.json must give each proposed claim a verdict of accepted, downgraded, or rejected with reasons, and must separately include substantive_progress as a boolean, substantive_progress_reason, and frontier_change. It may also contain a resolution candidate only for a faithful certified proof or directly verified counterexample of the exact problem; write resolution.json with kind, claim_id, and problem_statement_version only in that case. A reviewed informal proof is not an automatic resolution. Do not update state.md or history/. Respond with ONLY a JSON status envelope whose context_updates set context.assessment.summary and context.assessment.resolution_candidate.
