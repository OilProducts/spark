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
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: gate_item_completion
  label: Complete
  condition: outcome=success
  extensions:
    loop_restart: false
- to: implement_current
  label: Fix Validation
  condition: outcome=fail && preferred_label=Fix
  extensions:
    loop_restart: false
- to: implement_current
  label: Fix Validation Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: mark_current_blocked
  label: Blocked Validation
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: rewrite_current
  label: Rewrite Validation
  condition: outcome=fail && preferred_label=Rewrite
  extensions:
    loop_restart: false
- to: mark_current_blocked
  label: Blocked Validation Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
- to: rewrite_current
  label: Rewrite Validation Partial
  condition: outcome=partial_success && preferred_label=Rewrite
  weight: 1
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/validation-plan.json, .spark/spec-implementation/current/validation.stdout.txt, and .spark/spec-implementation/current/validation.stderr.txt. Also inspect context.tool.exit_code. Write .spark/spec-implementation/current/validation-result.json as valid JSON with fields item_id, item_title, status, command, exit_code, summary, and suites. Each suite entry must include name, status, required, summary, and skip_reason, where status is one of pass, fail, skipped, or blocked. Copy item_id and item_title from the current item and require them to match the current item context. Use .spark/spec-implementation/current/validation-plan.json as the expected validation scope and suite list. Treat skipped or blocked required suites as non-pass unless the item or run context explicitly allows that condition. Exit code 0 should normally map to pass when the captured evidence supports it; exit code 2 should be used for blocked validation when a missing prerequisite was identified; other nonzero exit codes should map to fail unless the captured evidence clearly shows a blocked external prerequisite. Fail validation if a stronger deterministic repo-wide gate was available but omitted, or if a changed documented public surface was checked only through helper-level or bypass paths. Respond with ONLY a JSON status envelope. Return outcome success only when .spark/spec-implementation/current/validation-result.json is written for the current item and status=pass with all required suites passing. If validation is non-pass because more implementation work is needed on the same item, return outcome fail with preferred_label Fix and write context.review.summary, context.review.required_changes, and context.review.blockers from the validation evidence. If validation is non-pass because the item is wrongly scoped or exposed prerequisite repair work that should be rewritten within this milestone, write milestone_dir/rewrite-items.json as a JSON array of replacement items using the existing item schema and return outcome fail with preferred_label Rewrite. If validation is non-pass only because it needs something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no honest substitute exists, write .spark/spec-implementation/current/block-reason.md and return outcome fail with preferred_label Blocked. Any other non-pass validation, including missing tools, dependencies, or repository support that can be installed or added, is Fix work.
