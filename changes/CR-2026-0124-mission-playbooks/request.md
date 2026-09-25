# Mission playbooks

## Summary

Add mission playbooks: named, reusable instructions for a kind of mission. A playbook gives the mission agent the broad shape of the work; the agent decides each step by inspecting run results and logs, and departs from the playbook when the evidence calls for it. Ship a bug report playbook first.

Make the analysis flows stop at their analysis, so a mission sequences the work instead of each flow gating and implementing on its own. Retire the flows the mission model makes redundant.

## Playbooks

- A playbook is a Markdown file with YAML frontmatter (`title`, `description`) and free-form instructions in the body.
- Installed playbooks live in `$SPARK_HOME/playbooks/<name>.md`. Bundled defaults live in `crates/spark-assets/assets/playbooks/` and are seeded the same way as starter flows: new files are created, and unmodified installed copies are updated.
- CLI: `spark playbook list` (name, title, description) and `spark playbook get --name <name>`.
- `spark mission create` accepts an optional `playbook` name, validated against the installed playbooks.
- On Start, the mission snapshots the playbook's name and text into its record, so later edits to the file do not change a running mission and the record shows what the mission followed.
- The mission frame pinned as system instructions includes the playbook text after the objective. Missions without a playbook keep today's frame.
- UI: the create form offers a playbook picker (optional). The transcript pane shows the playbook name under the pinned objective, with its text expandable.
- Update `spark-operations.md` and the workspace assistant frame: to run a playbook against an issue, list playbooks, create a mission with the playbook and the issue as its objective, and start it.

## Bug report playbook

Ship `bug-report.md` with these guidelines:

1. Reproduce the defect and find its root cause (`investigate-bug`).
2. Propose a fix and have it reviewed adversarially before any code changes (`investigate-bug`, which now verifies its plan).
3. Implement the fix with an adversarially reviewed, validated change (`implement-change`).
4. Verify independently: review the resulting branch (`review-change`), and check the original reproduction against it.
5. Merge (`merge-change`).
6. Close the mission with a summary of cause, fix, and evidence.

It also states that these are guidelines. Skip a step when the evidence already covers it (for example, the objective states an agreed fix). Repeat or reorder steps when results warrant. Read run results, and logs when results are not enough, before deciding. Ask the user only when a decision is theirs.

## Flows

Flows end with their result; the mission agent decides what happens next. Change the bundled copies under `crates/spark-assets/assets/flows/software-development/`:

- `investigate-bug`:
  - Keep investigate, verify diagnosis, and propose plan.
  - Add an adversarial plan-verification step that loops back to propose plan on failure, as diagnosis verification does.
  - Remove the approval gate and the `implement-change` subflow. The run ends with the verified diagnosis and plan.
- `design-change`: remove the approval gate and the implementation subflow. The run ends with the judged winning approach.
- `audit-codebase`: remove the approval gate and the backlog-fixing subflow. The run ends with the verified findings.
- `run-retrospective`: remove the approval gate and the implementation subflow. The run ends with the verified diagnosis and recommended fix.
- Update each flow's description to match.

Retire:

- `integrate-ready-branches`: a mission launching `merge-change` per branch replaces it. Delete the bundled flow and its default catalog entry.
- `implement-change-request`: `implement-change` with `artifact_path` pointing at a change request covers it, with worktree isolation that parallel runs need. It exists only as an installed flow. Remove its default catalog entry if one exists, and any references in the repo.

Unchanged: `implement-change`, `review-change`, `merge-change`, and the workers.

## Verification

- Seeding: bundled playbooks are created in an empty home, unmodified copies are updated, and edited copies are kept.
- CLI: `playbook list` and `get`; `mission create` with a valid playbook, and a clear error for an unknown one.
- Service: Start snapshots the playbook; the pinned frame contains the playbook text; a mission without a playbook keeps today's frame; editing the file after Start does not change the mission.
- Flows: each changed flow passes `spark flow validate`; `investigate-bug` has no human gate and no subflow, and loops on a failed plan verification; retired flows are gone from bundled assets and the default catalog.
- Frontend: playbook picker on create; playbook shown under the objective.
- `just test`.

## Out of scope

- Implement-spec and math research playbooks.
- Answering a human gate from the transcript.
- Deleting installed copies of retired flows from existing homes. Removing them is an operator step after this lands.
