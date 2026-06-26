# Agent Workflow Acceptance Tests

This directory contains high-level product workflow tests for Spark.

These files are not source-of-truth product specifications. They are acceptance assets derived from:
- `../../../specs/spark-ui-ux.md`
- `../../../specs/spark-workspace.md`

## Purpose

Use these workflows to verify that the UI works in practice for complete user goals, not just isolated component or API behavior.

They are bound to executable pytest cases by `harness.py` and `test_agent_workflows.py`. The harness drives Spark through local FastAPI product surfaces and deterministic local state, so it does not require external LLM/provider calls.

## Running

Run the harness from the repository root:

```bash
uv run pytest -q tests/acceptance/agent-workflows
```

The full repository gate also includes these cases:

```bash
uv run pytest -q
```

## Structure

Each workflow file should define:
- the user goal
- the required starting state
- the ordered steps through the live product
- the observable outcomes that determine pass/fail

The executable registry in `harness.py` maps every markdown workflow asset to a pytest case and declares the observable outcomes covered by that case. `test_every_markdown_workflow_asset_is_registered_in_harness` fails if a workflow markdown file is added without an executable case.

## Executable Coverage

- `project-select-author-execute-inspect.md`
  Registers a project, creates a project-scoped conversation, saves and reopens a flow, launches a run, inspects status/context/checkpoint/events/artifacts, edits the flow, and launches a follow-up run in the same project.
- `pipeline-author-workflow.md`
  Saves a structured graph/node/edge flow, verifies validation blocks invalid authoring, saves an edit, and reopens the edited flow through the workspace API.
- `operator-run-workflow.md`
  Launches a valid project run to terminal state and exercises an active run cancellation path with visible status transitions.
- `reviewer-auditor-workflow.md`
  Launches a completed run, discovers it in history, inspects status/context/checkpoint/journal data, and verifies artifact listing/download behavior.
- `project-owner-workflow.md`
  Sends a project-scoped chat turn with visible tool activity, creates and approves a flow-run request, verifies the launched run is recorded, then performs a direct launch that remains attached to the conversation event log.

## Notes

- These workflows should stay black-box and outcome-oriented.
- They should avoid implementation details unless a stable UI affordance is required for execution.
- They should be updated when the user-visible workflow changes materially.
