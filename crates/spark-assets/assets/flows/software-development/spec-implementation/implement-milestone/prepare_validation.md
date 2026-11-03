---
type: agent_task
title: Prepare Validation
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.item.target_paths
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate_repo
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: prepare_validation
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read .spark/state/spec-implementation/current/state.json only for request.validation_command, plus context.milestone.id, context.item.id, context.item.title, context.item.summary, context.item.target_paths, and the current milestone state in milestone_dir/state.json. Treat context.milestone.id and context.item.id as the authoritative live binding for the running worker, with milestone_dir/state.json as the durable ledger that must agree with them. Do not use shared runtime bindings under .spark/state/spec-implementation/current/current-milestone.json or state.json.current_milestone_id as the active milestone binding in this node, because runtime bookkeeping may legitimately drift while implementation is in flight. Write .spark/state/spec-implementation/current/run-milestone-validation.sh as a POSIX shell script that is itself the milestone validation entrypoint. Also write .spark/state/spec-implementation/current/validation-plan.json as valid JSON with fields item_id, item_title, entrypoint, resolved_command, scope_summary, and suites, where each suite includes name, required, and intent. Resolve validation in this order: request.validation_command from .spark/state/spec-implementation/current/state.json if present; otherwise the strongest deterministic repo-wide gate already supported by the repo; then narrower targeted checks as supplements. Prefer validation that is supportable from committed repository files and documented workflow. Do not rely on ambient tooling, one-off dependency injection, or test-only path hacks as the primary validation path. Do not edit repository deliverables in this node; implementation owns manifests, bootstrap files, package wiring, public exports, docs, and ignore rules. If truthful validation requires missing repository support, record the exact required work in validation-plan.json.scope_summary and make the validation script fail without running the expensive repo-wide gate so Assess Validation can route it back to implementation. Do not let narrow checks replace a stronger repo-wide gate. If the item changes a documented public surface, require at least one suite that exercises that surface directly rather than only helpers, fixtures, request builders, or manual transport code. Persist the resolved validation command back into .spark/state/spec-implementation/current/state.json once you establish it. The script must run the resolved or bootstrapped validation, write stdout directly to .spark/state/spec-implementation/current/validation.stdout.txt and stderr directly to .spark/state/spec-implementation/current/validation.stderr.txt as ordinary repo-local files rather than symlinks or indirections outside the repository root, and follow this exact exit-code protocol: exit 0 only when every required suite passed; exit 1 when any suite failed or locally repairable repository support is missing; exit 2 only when the script has positively identified a genuinely external missing prerequisite, such as credentials, access to a paid or third-party account, or physical hardware; and exit 3 for entrypoint or recording failures, such as missing validation-plan or item identity inputs or a failed artifact write, which are entrypoint errors rather than blocked validation. Run each suite command so its own exit status is captured rather than propagated: record that suite command's actual nonzero code in the existing validation artifacts (validation.stdout.txt or validation.stderr.txt) and normalize it to exit 1; never pass a suite command's exit code 2 through as an external blocker without that explicit diagnosis. Do not edit milestone_dir/state.json in this node. Do not write .spark/state/spec-implementation/current/validation-result.json in this node; the canonical active-workspace file remains .spark/state/spec-implementation/current/validation-result.json. Do not claim that a different item is now active. Choose `done` only after the validation script and validation plan are written. Respond with ONLY a JSON decision envelope.
