---
type: Flow
id: prove_refute
title: Prove or Refute
description: Attack a conjecture from both sides at once — a prover branch attempts a kernel-checked Lean proof while a refuter branch searches for machine-verified counterexamples — exchanging stuck lemmas and near-misses each round, with an arbiter that settles, repairs the statement, or continues, and honest budget-bounded synthesis into the dossier. Launch with execution_profile_id math-lab.
execution_lock:
  scope: project
  key: math-research
  conflict_policy: queue
schema_version: '1'
inputs:
- key: context.request.conjecture
  label: Conjecture
  type: string
  description: The conjecture to attack, stated precisely enough that a proof attempt and a counterexample search target the same claim.
  required: true
- key: context.request.notes
  label: Notes
  type: string
  description: Optional known results, definitions, constraints, or prior-session pointers worth starting from.
  required: false
- key: context.request.max_rounds
  label: Max Rounds
  type: string
  description: Maximum number of prove/refute rounds before the session synthesizes what it has (default 3).
  required: false
defaults:
  fidelity: summary:high
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  math_research:
    launcher: true
    required_execution_profile: math-lab
---

Settle the stated conjecture — prove it, refute it, or repair it — with every claim backed by a machine-checked artifact, and record what the session learned in the dossier.
