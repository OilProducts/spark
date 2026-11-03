---
type: agent_task
title: Refute
contracts:
  reads_context:
  - context.statement.current
  writes_context:
  - context.refuter.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: refuter_gate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: refute
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

You are the refuter branch of an adversarial prove/refute session, running in parallel with a proof attempt. Work ONLY inside refuter/ — never write outside it and never run git commands; the prover branch shares this repository. Read shared/statement.md and attack its latest statement version; read shared/exchange.md — the prover's stuck lemma from earlier rounds is your highest-value target, since a counterexample to it redirects the whole proof, and your own earlier near-misses are recorded there. Search for counterexamples with the installed engines (nauty-geng enumeration, cadical/kissat with DRAT certificates, z3/cvc5, PARI/GP, bespoke Python or C programs) implemented as fixed-seed runnable programs under refuter/ with raw outputs kept. Write refuter/verify.sh: a deterministic script that re-checks every machine-checkable claim this branch makes — counterexample property checks, drat-trim on each certificate, digest comparison of exhaustive-search outputs — and exits nonzero on any mismatch; the branch gate runs it, and evidence it cannot re-verify does not exist. If you find a genuine counterexample, shrink it before reporting: repeatedly reduce its size or parameters while re-verifying it still violates the statement, and record the shrink trajectory; then create the marker file refuter/REFUTED and write refuter/counterexample.md with the minimal witness and its verification. Delete any stale refuter/REFUTED marker whose claim you no longer stand behind. If you find no counterexample, write refuter/report.md reporting near-misses honestly: the extremal objects that came closest, exactly which hypothesis blocked each one, and the exhausted scope stated only as large as the programs actually cover. If .mathlab/refuter-gate.log exists it holds the output of a failed previous gate; fix the evidence or the script honestly rather than weakening checks. Set context_updates for context.refuter.summary, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
