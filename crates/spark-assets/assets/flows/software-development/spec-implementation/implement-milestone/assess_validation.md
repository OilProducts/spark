---
type: agent_task
title: Assess Validation
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.tool.exit_code
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: mark_current_blocked
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  complete:
    to: gate_item_completion
    description: The current item or milestone satisfies this node’s completion criteria with all required evidence and ledger updates; no in-scope work remains for that unit.
  fix:
    to: implement_current
    description: The completed review or validation assessment found concrete defects requiring another implementation pass on the same item; record actionable findings.
  rewrite:
    to: rewrite_current
    description: The current item or milestone decomposition is unsuitable; write the required replacement-work ledger before rewriting it.
  unfinished:
    to: assess_validation
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read .spark/state/spec-implementation/current/validation-plan.json, .spark/state/spec-implementation/current/validation.stdout.txt, and .spark/state/spec-implementation/current/validation.stderr.txt. Also inspect context.tool.exit_code. Write .spark/state/spec-implementation/current/validation-result.json as valid JSON with fields item_id, item_title, status, command, exit_code, summary, and suites. Each suite entry must include name, status, required, summary, and skip_reason, where status is one of pass, fail, skipped, or blocked. Copy item_id and item_title from the current item and require them to match the current item context. Use .spark/state/spec-implementation/current/validation-plan.json as the expected validation scope and suite list. Treat skipped or blocked required suites as non-pass unless the item or run context explicitly allows that condition. context.tool.exit_code is the generated validation entrypoint's own exit code: 0 for pass, 1 for any failed suite or missing locally repairable support, or 2 for a positively identified external prerequisite. Exit code 0 should normally map to pass when the captured evidence supports it; exit code 2 should map to blocked validation only when the captured evidence confirms the missing prerequisite is genuinely external; exit code 1 should map to fail unless the captured evidence clearly shows a blocked external prerequisite. The exit code alone never decides this node's decision; judge the captured evidence independently. Fail validation if a stronger deterministic repo-wide gate was available but omitted, or if a changed documented public surface was checked only through helper-level or bypass paths. Choose `complete` only when .spark/state/spec-implementation/current/validation-result.json is written for the current item and status=pass with all required suites passing. If validation is non-pass because more implementation work is needed on the same item, choose `fix` and write context.review.summary, context.review.required_changes, and context.review.blockers from the validation evidence. If validation is non-pass because the item is wrongly scoped or exposed prerequisite repair work that should be rewritten within this milestone, write milestone_dir/rewrite-items.json as a JSON array of replacement items using the existing item schema and choose `rewrite`. If validation is non-pass only because it needs something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no honest substitute exists, write .spark/state/spec-implementation/current/block-reason.md and choose `blocked`. Any other non-pass validation, including missing tools, dependencies, or repository support that can be installed or added, is `fix` work. Respond with ONLY a JSON decision envelope.
