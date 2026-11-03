---
type: agent_task
title: Prove
contracts:
  reads_context:
  - context.statement.current
  writes_context:
  - context.prover.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: prover_gate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: prove
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

You are the prover branch of an adversarial prove/refute session, running in parallel with a counterexample search. Work ONLY inside prover/ — never write outside it and never run git commands; the refuter branch shares this repository. Read shared/statement.md and attack its latest statement version; read shared/exchange.md — the refuter's near-misses from earlier rounds are your case analysis (they show where the obstruction lives), and your own earlier stuck points are recorded there. Attempt a proof: informal argument first, then formalize what you can in Lean 4 under prover/lean (copy /opt/mathlab/template to prover/lean if it does not exist; Mathlib is prebuilt; never modify /opt/mathlab/mathlib4). Follow AGENTS.md proof-integrity rules exactly. Create the marker file prover/PROVED if and only if you hold a complete kernel-checked proof of the full latest statement — (cd prover/lean && lake build) succeeds with no sorry, no admit, and no new axioms; delete any stale prover/PROVED marker whose claim you no longer stand behind. Either way write prover/report.md: status, method, lemmas actually proved with their Lean names, and — most importantly when stuck — the precise statement of the smallest lemma you cannot prove, stated so the refuter can attack it directly next round. If .mathlab/prover-gate.log exists it holds the output of a failed previous gate; fix the proof honestly or withdraw the marker rather than weakening the statement silently. Set context_updates for context.prover.summary, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
