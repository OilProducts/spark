---
type: agent_task
title: Draft Design
role: author
runtime:
  session: designer
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.request.constraints
  - context.request.design_path
  - context.request.artifact_path
  writes_context:
  - context.review.contested
  - context.design.path
  - context.design.summary
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: review
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: draft
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Write a design document for the change in context.request.objective, respecting context.request.constraints and weighting context.request.target_paths. If context.request.artifact_path is present, adopt that existing design instead of drafting a new one: bring it up to the standard below and keep its path. Otherwise write the design to context.request.design_path; when that is absent, follow the project's own instructions about where designs or change requests live, including their numbering rules, and when the project has none, use docs/designs/<slug>.md.

Independent reviewers will read only this document and the files it references, so it must stand on its own. It has these sections:
- Problem.
- Goals and non-goals.
- Approach.
- Alternatives considered: at least one other viable approach and why it lost.
- Risks.
- Implementation plan: the target files, each marked create, modify or delete; ordered steps, each leaving the code buildable and testable; and the test plan. Write the smallest design that meets the request: decide every choice that changes behaviour or scope, and leave routine mechanics to the implementer.
- Open questions.

Before writing, verify every file, symbol, behaviour and line you cite against the current code. When the user's intent is unclear, ask the run's owner through the clarification tool rather than guessing. Do not change any file other than the design. Choose `done` when the design is written, with context_updates setting context.design.path (the design's path relative to the working directory) and context.design.summary (a few sentences on the problem and the chosen approach). Respond with ONLY a JSON decision envelope.
