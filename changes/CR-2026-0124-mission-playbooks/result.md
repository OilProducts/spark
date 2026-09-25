---
id: CR-2026-0124-mission-playbooks
title: Mission playbooks
status: completed
type: feature
changelog: public
---

# Mission playbooks: result

## Summary

Missions can now follow a named playbook: a reusable set of instructions for a kind of mission. The first bundled playbook is `bug-report`. The analysis flows now end with their analysis, and the mission decides what to do next. `integrate-ready-branches` is retired.

- **Playbooks.** A playbook is a Markdown file with `title`/`description` frontmatter, stored at `$SPARK_HOME/playbooks/<name>.md`.
  - Server startup seeds the bundled playbooks the same way it seeds starter flows. It tracks installed copies in `.packaged-playbook-hashes.json`, updates copies the user hasn't modified, and keeps edited ones.
  - The HTTP API serves `GET /workspace/api/playbooks` and `GET /workspace/api/playbooks/{name}`.
  - The CLI adds `spark playbook list` and `spark playbook get --name <name>`.
- **Missions.**
  - `playbook` is an optional mission field. On create or edit, it is checked against the installed playbooks. A started mission can't change its playbook.
  - On Start, the mission copies the playbook's name and text into its record.
  - The pinned mission frame shows the playbook text after the objective. Missions without a playbook keep the previous frame.
- **UI.**
  - The mission create form has an optional playbook picker.
  - The transcript pane shows `Playbook: <name>` under the objective, with the text in an expandable section. A draft shows only the name it will use.
- **Bug report playbook.** `bug-report.md` lists the six requested steps: investigate, verified plan, implement, independent review plus reproduction check, merge, close. It also says these are guidelines, not a fixed script.
- **Flows.**
  - `investigate-bug` now ends with a verified diagnosis and plan. A new `verify_plan` step returns to `propose_plan` on failure. The approval gate and the `implement-change` subflow are removed.
  - `design-change`, `audit-codebase` and `run-retrospective` no longer have their approval gate or their implementation/backlog subflow.
  - All four flow descriptions were updated to match.
- **Retired flows.**
  - `integrate-ready-branches.yaml` is deleted from bundled assets. It is also removed from `DEFAULT_AGENT_REQUESTABLE_FLOWS` and from the integration execution lock, which now covers only `merge-change`.
  - `implement-change-request` had no default catalog entry. Its mention in `AGENTS.md` was reworded.
- **Docs.** `spark-operations.md` and the workspace assistant frame now explain how to run a playbook against an issue. `docs/first-flow-tutorial.md` was also updated.

## Validation

- `just test` (fmt check, all Rust tests, frontend unit tests, frontend build) exited 0 on the current tree.
  - The first full run failed once in `attractor-api` `pipeline_lifecycle_contracts::workflow_executes_with_captured_profile_after_profile_file_changes`. The error was `WouldBlock` on a mock-server socket read, in code this change doesn't touch.
  - That test passed when rerun alone, and the second full `just test` run passed.
- `spark flow validate --file` returned `status: ok` for `audit-codebase`, `design-change`, `investigate-bug` and `run-retrospective`.
- `npm --prefix frontend run ui:smoke -- missions-editor` passed 4 of 4, run after the frontend build from `just test`.
- New tests:
  - `resource_contracts::analysis_flows_end_with_their_result_and_investigate_bug_verifies_its_plan` checks that the four flows have no human gate or subflow, and that `investigate-bug` has the `propose_plan → verify_plan → done` edges and the fail loop back to `propose_plan`.
  - `server_shell_contracts::init_seeds_playbooks_updating_unmodified_copies_and_keeping_edits`.
  - `trigger_cli_m4_route_contracts::playbook_cli_lists_gets_and_validates_mission_playbooks` covers `list`, `get`, and `mission create` with both a valid and an unknown playbook.
  - `mission_contracts::start_snapshots_the_playbook_into_the_record_and_the_frame` covers the snapshot, frame placement after the objective, the previous frame when there is no playbook, and that editing the file after Start changes nothing.
  - `MissionsPanel.test.tsx` covers creating with a picked playbook and showing it under the objective with expandable text.
- Not done: no live mission ran the `bug-report` playbook end to end with a real agent.

## Shipped Changes

- Playbooks: `crates/spark-workspace/src/playbooks.rs` (new), `crates/spark-assets/assets/playbooks/bug-report.md` (new), `crates/spark-assets/src/lib.rs`, `crates/spark-server/src/lib.rs` (seeding), `crates/spark-http/src/workspace.rs` (routes), `crates/spark-cli/src/lib.rs`, `crates/spark-cli/src/output.rs`.
- Missions: `crates/spark-workspace/src/missions.rs`, `crates/spark-workspace/src/lib.rs`, `crates/spark-workspace/src/conversations.rs`.
- Flows and catalog: `crates/spark-assets/assets/flows/software-development/{investigate-bug,design-change,audit-codebase,run-retrospective}.yaml`; `integrate-ready-branches.yaml` deleted; `crates/spark-storage/src/workspace_flow_catalog.rs`.
- Frontend: `frontend/src/features/missions/{MissionEditor,MissionDetail,MissionsPanel}.tsx`, `frontend/src/features/missions/__tests__/MissionsPanel.test.tsx`, `frontend/e2e/smoke/missions-editor.spec.ts`.
- Docs: `crates/spark-assets/assets/guides/spark-operations.md`, `docs/first-flow-tutorial.md`, `AGENTS.md`.
- Tests: `crates/spark-assets/tests/contracts/resource_contracts.rs`, `crates/spark-cli/tests/process_contracts/{cli_process_contracts,cli_shell_contracts,trigger_cli_m4_route_contracts}.rs`, `crates/spark-server/tests/process_contracts/server_shell_contracts.rs`, `crates/spark-storage/tests/contracts/workspace_flow_catalog_contracts.rs`, `crates/spark-workspace/tests/contracts/{mission_contracts,review_artifact_contracts}.rs`.
- `.gitignore` un-ignores this change request directory.

Notes:

- The string `implement-change-request` still appears in test data only: `crates/test-fixtures/compat/sse/*.json` trigger fixtures, and an arbitrary path in a catalog seeding test. Neither loads the flow.
- As the request's out-of-scope section says, existing homes keep their installed copies of `integrate-ready-branches.yaml` and `implement-change-request.yaml` until an operator removes them.
- The changes are uncommitted in the working tree on `main`.
