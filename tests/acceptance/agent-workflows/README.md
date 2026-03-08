# Agent Workflow Acceptance Tests

This directory contains high-level product workflow tests for Spark Spawn.

These files are not source-of-truth product specifications. They are acceptance assets derived from:
- `/Users/chris/tinker/sparkspawn/specs/ui-spec.md`
- `/Users/chris/tinker/sparkspawn/specs/ui-user-stories.md`
- `/Users/chris/tinker/sparkspawn/specs/ui-story-records.md`

## Purpose

Use these workflows to verify that the UI works in practice for complete user goals, not just isolated component or API behavior.

They are intended to evolve into executable agent-driven acceptance tests once the required computer-use harness exists.

## Structure

Each workflow file should define:
- the user goal
- the required starting state
- the ordered steps through the live product
- the observable outcomes that determine pass/fail

## Current Workflows

- `project-select-author-execute-inspect.md`
  End-to-end Home -> Editor -> Execution -> Runs workflow.
- `pipeline-author-workflow.md`
  Author-focused flow editing and validation workflow.
- `operator-run-workflow.md`
  Execution launch, monitoring, and cancellation workflow.
- `reviewer-auditor-workflow.md`
  Run inspection and diagnostic reconstruction workflow.
- `project-owner-workflow.md`
  Project-scoped conversation, spec-edit review, and execution-card workflow.

## Notes

- These workflows should stay black-box and outcome-oriented.
- They should avoid implementation details unless a stable UI affordance is required for execution.
- They should be updated when the user-visible workflow changes materially.
