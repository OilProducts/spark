---
type: agent_task
title: Experiment
contracts:
  reads_context:
  - context.survey.plan
  - context.review.required_changes
  writes_context:
  - context.experiments.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: verify_evidence
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: experiment
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Execute the experiment plan from context.survey.plan. Implement each experiment under experiments/ as runnable programs with fixed seeds and exact inputs; run them; keep raw outputs, and for every SAT result keep the DRAT certificate beside its CNF. Write experiments/verify.sh: a deterministic script that re-checks all machine-checkable evidence (drat-trim on each certificate, digest comparison of exhaustive-search outputs, re-execution of cheap checks) and exits nonzero on any mismatch — the next node runs it as a gate, and evidence it cannot re-verify does not exist. If .mathlab/verify.log exists it holds the output of a failed previous verification; fix the evidence or the script honestly rather than weakening checks. If context.review.required_changes is present, close those gaps. Record an experiment log in experiments/log.md. Set context_updates for context.experiments.summary, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
