---
type: agent_task
title: Experiment
contracts:
  reads_context:
  - context.survey.plan
  - context.review.required_changes
  writes_context:
  - context.experiments.summary
  response: status_envelope
routes:
- to: verify_evidence
---

Execute the experiment plan from context.survey.plan. Implement each experiment under experiments/ as runnable programs with fixed seeds and exact inputs; run them; keep raw outputs, and for every SAT result keep the DRAT certificate beside its CNF. Write experiments/verify.sh: a deterministic script that re-checks all machine-checkable evidence (drat-trim on each certificate, digest comparison of exhaustive-search outputs, re-execution of cheap checks) and exits nonzero on any mismatch — the next node runs it as a gate, and evidence it cannot re-verify does not exist. If .mathlab/verify.log exists it holds the output of a failed previous verification; fix the evidence or the script honestly rather than weakening checks. If context.review.required_changes is present, close those gaps. Record an experiment log in experiments/log.md. Respond with ONLY a JSON status envelope whose context_updates set context.experiments.summary.
