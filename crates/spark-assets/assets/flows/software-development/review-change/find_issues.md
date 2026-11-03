---
type: agent_task
title: Find Issues
contracts:
  reads_context:
  - context.request.source_ref
  - context.request.objective
  writes_context:
  - context.findings.list
  - context.findings.scope
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: verify_findings
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: find_issues
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Review the change named by context.request.source_ref without modifying any files. Resolve the diff against the merge base with the current checkout (git merge-base HEAD <ref>), then reconstruct the change's apparent contract: what it sets out to do, its acceptance criteria, and constraints it must not break. Judge the actual diff and repository state, not commit messages or summaries. Weight context.request.objective focus areas when present. Look for: correctness defects on the real execution path, partial implementations, dead or uncalled code, mocks masquerading as functionality, hardcoded happy paths, regressions in adjacent surfaces, cut validation or safety, and minimality problems (speculative abstraction, reinvented stdlib or existing code, future-proofing beyond the task) — flagging minimality only when it creates risk, behavior change, or significant complexity. For every finding record: the file and location, the defect claim, how it fails concretely, and an evidence grade of strong (deterministic check, direct inspection), medium (clear code-path reading), or weak (suspicion). Choose `done` when the review is complete, whether or not it found issues, with context_updates setting context.findings.list (the findings with locations, claims, and evidence grades) and context.findings.scope (the resolved ref, merge base, and files reviewed). Choose `blocked` only when you cannot perform the review at all, such as when the ref cannot be resolved. Respond with ONLY a JSON decision envelope.
