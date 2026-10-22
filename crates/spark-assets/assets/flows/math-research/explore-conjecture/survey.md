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
  response: status_envelope
routes:
- to: experiment
---

Read DOSSIER.md first per AGENTS.md dossier discipline: do not re-derive results it records, and do not re-run attacks it records as failed without a new reason. Then survey the problem stated in context.request.problem (narrowed by context.request.focus when present) without running experiments yet. Restate the problem precisely, record what is known — including whether the problem or nearby cases are already resolved in the literature, labeled per AGENTS.md rules — and design a small set of concrete computational experiments the installed engines can run (SAT via cadical/kissat with DRAT certificates, SMT via z3/cvc5, graph enumeration via nauty-geng, number theory via PARI/GP, optimization via CBC/CSDP, bespoke programs in Python or C). Write the survey and the experiment plan to survey.md. Respond with ONLY a JSON status envelope whose context_updates set context.survey.summary and context.survey.plan.
