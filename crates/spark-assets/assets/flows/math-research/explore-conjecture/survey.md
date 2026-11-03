---
type: agent_task
title: Survey
contracts:
  reads_context:
  - context.request.problem
  - context.request.focus
  writes_context:
  - context.survey.summary
  - context.survey.plan
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: experiment
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: survey
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read DOSSIER.md first per AGENTS.md dossier discipline: do not re-derive results it records, and do not re-run attacks it records as failed without a new reason. Then survey the problem stated in context.request.problem (narrowed by context.request.focus when present) without running experiments yet. Restate the problem precisely, record what is known — including whether the problem or nearby cases are already resolved in the literature, labeled per AGENTS.md rules — and design a small set of concrete computational experiments the installed engines can run (SAT via cadical/kissat with DRAT certificates, SMT via z3/cvc5, graph enumeration via nauty-geng, number theory via PARI/GP, optimization via CBC/CSDP, bespoke programs in Python or C). Write the survey and the experiment plan to survey.md. Set context_updates for context.survey.summary and context.survey.plan, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
