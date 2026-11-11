---
type: agent_task
title: Repository Integrity Audit
runtime: {}
contracts:
  reads_context:
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  cleanup:
    to: plan_cleanup_milestone
    description: Repository integrity or deliverability gaps remain; write the cleanup ledger so bounded cleanup milestones can be planned.
  deliverable:
    to: done
    description: The repository passes the final deliverability and integrity criteria with no outstanding bound contract decision.
  unfinished:
    to: repository_integrity_audit
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/source.md, <internal.project_work_dir>/spec-implementation/current/spec/requirements.json, <internal.project_work_dir>/spec-implementation/current/milestones.json, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, and the current repository state as needed. Decide whether the repository is now honestly deliverable, not merely internally coherent. Check that the repository can be built, run, imported, and validated from committed metadata, manifests, bootstrap files, docs, public exports, and the approved contract decisions when applicable; that tests are behavioral rather than brittle source-text or path-hack checks; that tracked local, generated, cache, IDE, or Spark workflow artifacts are not part of the deliverable; that docs, exports, manifests, and ignore rules stay coherent; and that no obvious duplicate validators or mixed-responsibility god files remain as the final state. Success requires a clear source of truth for delivered surfaces, no unjustified wrapper-only duplicate layers, essential repo metadata and docs for the technologies in use, and no bound contract decision left satisfied only through weaker internal or metadata-only paths. If the repository is deliverable, choose `deliverable`. If further cleanup/refactor work can close the remaining gaps, write <internal.project_work_dir>/spec-implementation/current/repository-integrity-gaps.md with the missing work. The cleanup ledger is <internal.project_work_dir>/spec-implementation/current/repository-integrity-gaps.md. Then choose `cleanup`. Large or redesign-level gaps are still `cleanup` work: describe them in the cleanup ledger so they can be planned as one or more cleanup/refactor milestones. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Respond with ONLY a JSON decision envelope.
