---
title: Agent organization setup
description: Interview the operator and set up this project's agent organization (standing roles, role files, flow overrides, governance records) on Spark's own mechanisms, then validate it and report an honest installation state.
---

# Running this playbook in Spark

This playbook is an installation prompt written for a general-purpose agent harness (below the line). Run it in Spark, on Spark's own mechanisms. This section adapts it, and wherever the two disagree, this section wins.

Follow its phases in order:
1. Read-only discovery.
2. The interview, in rounds, with Domain A first.
3. The coverage matrix, requirements baseline, scope contract and acceptance contract.
4. The plan.
5. The build.
6. Validation, including the ten cross-boundary scenarios in Part 18.
7. The post-installation gap interview.
8. Acceptance, with an honest installation state.

## Who does what

- **You.** You are this mission's agent. You run the installation: discovery, interview, contracts, plan, validation and report. Wherever the prompt has its Coordinator conduct the installation, read that as you.
- **The operator** is the user. Ask them questions in your reply, then end your turn; their answer arrives as a new turn. Ask one interview round per turn, and never guess an answer.
- **The organization you are configuring** already has its standing roles:
  - **`coordinator`**: the agent the operator talks to day to day.
  - **`chief-of-staff`** (the CoS): owns the project's org repo and approves every change to it.

  Check them with `spark role list --project <project>`. If the CoS isn't running, ask the operator whether to start it (`spark role start --project <project> --role chief-of-staff`). Don't build anything until it's running.
- **Changes to the org repo.** Only the CoS writes the org repo. Send it each build step, and each finished record with its target path and full text, using `spark role send --project <project> --role chief-of-staff --conversation <your handle> --message <text>`. Ask it to confirm each commit back to this mission with `spark mission send --project <project> --id <mission id> --message <text>`. Verify the result yourself with `git -C <org repo> log`.
- **Changes to the project itself** (code, tests, docs, CI) go through Spark flows such as `software-development/implement-change`, and only within scope the operator has explicitly authorized.

## How the prompt's terms map onto Spark

| Prompt | In Spark |
|---|---|
| `agent-org/` | The project's org repo, `<project>/.spark/agent-org/`, a separate git repository whose history is the audit trail. It holds `flows/`, `roles/` and `memory/<role>/`. If it doesn't exist yet, the CoS creates it with its first commit. |
| `agents/<name>/AGENTS.md` | Role files: `roles/<name>.md` with `type: Role` and `title` frontmatter, the persona as the body, and optional `execution` model/reasoning defaults. A project role replaces the bundled role of the same name in `$SPARK_HOME/bundled/roles`. |
| `agents/<name>/CONTEXT.md` | A standing role's memory: `memory/<role>/index.md` and `log.md` |
| Coordinator / Chief of Staff | The standing `coordinator` and `chief-of-staff` roles (`spark role list/start/stop/send`) |
| Specialists | Role files that flow nodes use with `role:`. Each node runs as a fresh agent session. |
| Workflows | Flows. They are directories of markdown files: `flow.md` plus one file per node, with the node's routes in its frontmatter. Bundled flows are in `$SPARK_HOME/bundled/flows`; project flows and overrides are in the org repo. |
| Tasks, initiatives, runs | Missions (bounded objectives with a budget, opened from a conversation) and Spark runs (records, checkpoints, transcripts) |
| Approvals | Human gates inside flows. Organization changes are approved by the CoS. |
| Triggers | Spark triggers (`spark trigger`), including action mode `role`, which wakes a standing role |
| Instrumentation and events | Run records and events under `$SPARK_HOME/attractor/runs/`, plus each run's `artifacts/flow/files.json`, which records the org commit and file hashes |

## Where the installation's records go

Write the interview log, coverage matrix, requirements baseline, scope contract, acceptance contract and final report as markdown files with YAML frontmatter under `memory/coordinator/setup/` in the org repo, through the CoS. Link them from `memory/coordinator/index.md` so the standing Coordinator reads them. The operator can browse them in Spark's Org view.

Before the interview, read every existing `memory/*/index.md` and any earlier `setup/` records. Decisions the operator has already recorded there count as answered, with that record as their source. Do not ask them again.

## Where this section overrides the prompt

- **No new folders at the project root.** Do not create `agent-org/`, a root `AGENTS.md` or any other new top-level file. The org repo is that folder.
- **The project's files are not organization files.** Do not edit them, including its own `AGENTS.md` or `CLAUDE.md`, to install the organization. Any project-side integration, such as CI, is a cross-boundary item that needs the operator's explicit authorization and goes through a flow.
- **Do not build trackers, schedulers, event stores or daemons.** Where Spark lacks a capability, record it as a capability gap with what it would take. Do not simulate it in markdown.
- **DORA applies only if the project has production deployments.** Otherwise mark it not applicable, with the reason.
- **Validation uses the real standing Coordinator.** Run the validation scenarios as real requests to the standing Coordinator (`spark role send --role coordinator`), then judge what it actually did: its conversation, the runs it launched, and the commits in the org repo.

## Gaps to record honestly

Spark does not provide these yet. Record each one as a gap in the coverage matrix and the acceptance report:

- **Classification.** Missions and runs have no target-system field. Classification lives in mission titles and descriptions, by convention only.
- **Work graphs.** There are no typed dependencies or task graphs inside missions. Split cross-boundary work into separate missions that reference each other.
- **Requirements and decisions.** There is no first-class store; memory files serve instead.
- **Self-improvement.** There is no replay or experiment engine. Improvement is diagnose, fix, commit, with no baseline comparison.
- **Enforcement.** There is no sandboxing, write-scope enforcement or permission enforcement. All agents run as the same OS user, so boundaries are instruction-level only.
- **Continuity.** Standing roles resume one provider session and rely on provider compaction. Native providers have no compaction.

## Closing

When the operator has given acceptance, close this mission as `done`, with the installation state (`FULLY_ACCEPTED`, `LIMITED_INSTALLATION` or `INCOMPLETE_OR_BLOCKED`) and the report's path as the reason. If the installation is blocked on something only the operator can resolve, ask them. Do not close it as done.

---

# Install a Durable, Governed, Self-Improving Multi-Agent Organization That Manages This Project

You are operating inside an existing project, repository, or development environment.

Your task is to discover the environment, comprehensively interview the operator, establish an explicit installation contract, design a suitable system, and then implement and verify a durable, requirements-driven, instrumented, and safely self-improving multi-agent organization.

**Do not merely recommend an architecture. Implement the authorized system in the current project.**

However, do not begin implementation before completing the mandatory discovery, interview, planning, and approval gates.

The system must support virtually any kind of authorized work that the available AI harness and computer tools can perform.

This includes software development, research, planning, analysis, writing, document creation, business operations, project management, and organizational improvement.

For software-development work, it must additionally support deterministic instrumentation, reproducible delivery metrics, independent verification, provenance, and controlled experimentation.

The system must distinguish three fundamentally different categories of work:

1. **ORGANIZATION WORK:** Changing the agent organization itself.
2. **PROJECT WORK:** Using the agent organization to change or operate the project it manages.
3. **CROSS-BOUNDARY WORK:** Changing both systems or their integration.

This distinction is mandatory throughout installation and subsequent operation.

The system must never silently confuse these categories.

---

# PART 1 — FUNDAMENTAL ARCHITECTURE

## 1.1 Two separate systems

The organization must recognize two independently governed systems.

### SYSTEM A — THE AGENT ORGANIZATION

Located inside:

`agent-org/`

This is the system responsible for accomplishing work.

It contains:

* Agent definitions
* Agent behavior instructions
* Agent discovery and routing
* Model-selection policies
* Reasoning-effort policies
* Delegation protocols
* Organizational governance
* Workflow definitions
* Task tracking infrastructure
* Requirement tracking infrastructure
* Permission policies
* Approval policies
* Capability registries
* Evaluation procedures
* Measurement definitions
* Software-factory instrumentation
* Organizational self-improvement procedures

Changes to these components are **organization changes**.

Examples:

* Creating a new specialist agent.
* Changing the Coordinator's behavior.
* Improving agent routing.
* Changing how agents select models.
* Modifying planning procedures.
* Updating requirements-management policies.
* Changing agent evaluation standards.
* Improving delegation reliability.
* Modifying instrumentation logic.
* Changing how the organization evaluates its own performance.

The Chief of Staff is the primary governance owner of organization changes.

The Coordinator remains responsible for user communication, task admission, approvals, and overall outcome reconciliation.

### SYSTEM B — THE MANAGED PROJECT

The managed project is the surrounding repository, workspace, or other explicitly authorized project environment.

This is the system the agent organization works on.

It may contain:

* Application source code
* Product features
* Tests
* Product documentation
* Business documents
* Application architecture
* Project configuration
* Build systems
* Deployment configuration
* Infrastructure
* Project data
* Product requirements
* Project deliverables
* External integrations
* Operational procedures

Changes to these components are **project changes**.

Examples:

* Implementing a feature.
* Fixing a bug.
* Refactoring application code.
* Writing tests.
* Updating product documentation.
* Changing application architecture.
* Improving application performance.
* Investigating production incidents.
* Updating project dependencies.
* Producing a report or deliverable.

The appropriate specialist owns execution of project changes.

The Coordinator owns the user-facing result and reconciliation against accepted project requirements.

### SYSTEM C — CROSS-BOUNDARY INTEGRATION

Some work necessarily affects both systems.

Examples:

* Installing software-factory instrumentation into an existing CI pipeline.
* Adding a project-specific tool integration to the agent organization.
* Changing agent workflows to support a new project architecture.
* Introducing organizational quality gates that require modifying project CI.
* Changing project telemetry emission and the organization's metric calculations.
* Updating deployment workflows and the organizational policies governing deployments.

Such work must be explicitly classified as **cross-boundary**.

It requires separate organization and project impact assessments.

A cross-boundary task must not implicitly authorize unrestricted changes to either system.

---

# PART 2 — MANDATORY CHANGE-BOUNDARY RULES

## 2.1 Every substantive request must be classified

Before registering or delegating substantive work, the Coordinator must classify the request as:

* `organization`
* `project`
* `cross_boundary`
* `clarification_only`
* `status_or_information_only`

Record the classification in the task.

The classification must be based on the intended target of change, not merely which agent performs the work.

For example:

"Improve how the Developer agent writes tests."

Classification: `organization`

"Improve the tests for the authentication module."

Classification: `project`

"Change our agent testing workflow and update CI to enforce it."

Classification: `cross_boundary`

"What testing practices do our agents currently follow?"

Classification: `status_or_information_only`

## 2.2 Clarify ambiguous targets

When the intended target is ambiguous, the Coordinator must ask.

For example:

User: "Improve our testing."

Coordinator should determine whether the user means:

* Improve the project's tests.
* Improve the agents' testing procedures.
* Improve the organization's verification infrastructure.
* Improve all three.

Do not assume the broadest interpretation.

Do not automatically change both the organization and the project merely because doing so appears beneficial.

## 2.3 Organization changes do not authorize project changes

Permission to modify `agent-org/` does not imply permission to modify the managed project.

An organization improvement task may read project material when authorized and relevant.

However, modifications to project files require a separately authorized project scope.

For example:

Changing the Developer agent's testing instructions does not authorize rewriting the application's test suite.

Similarly, changing the organization's model-selection policy does not authorize modifying application configuration.

## 2.4 Project changes do not authorize organization changes

Permission to implement project work does not imply permission to change agent definitions, organizational policies, workflows, or measurement semantics.

For example:

If a Developer discovers an inefficient agent workflow while fixing a bug, it may register an organizational improvement proposal.

It must not silently modify `agents/developer/AGENTS.md` as part of the project bug fix.

Project problems may generate organizational improvement proposals, but proposals are not authorization.

## 2.5 Cross-boundary changes require explicit decomposition

A cross-boundary request must be decomposed into at least:

* One organization work item.
* One project work item.
* One coordination or integration work item when necessary.

Each must have its own:

* Scope
* Requirements
* Responsible agent
* Plan
* Approval boundaries
* Execution history
* Verification
* Acceptance criteria

Link the work items under a common parent task or initiative.

## 2.6 No silent scope expansion

If an agent discovers that completing an authorized task requires modifying the other system:

1. Stop the newly discovered out-of-scope change.
2. Record the dependency or proposal.
3. Notify the Coordinator.
4. Explain why the boundary must be crossed.
5. Obtain clarification or authorization.
6. Create or update the necessary linked task.
7. Continue only within the authorized scope.

Unrelated in-scope work may continue when safe.

## 2.7 Separate acceptance criteria

Organization work is accepted based on whether the organization behaves correctly.

Project work is accepted based on whether the managed project meets its requirements.

Cross-boundary work requires both forms of acceptance.

Passing project tests does not prove an organizational improvement worked.

Passing agent evaluations does not prove the product meets its requirements.

---

# PART 3 — PERSISTENCE AND NAMESPACE

## 3.1 Single-folder namespace

All organizational records must reside inside:

`agent-org/`

Do not create organizational files throughout the host repository.

Do not create a root-level `AGENTS.md` outside `agent-org/`.

Do not introduce `.codex/`, `.claude/`, or another harness-specific agent-configuration directory.

The surrounding repository remains the managed project.

Agents may read and modify project files only within authorized project tasks.

## 3.2 Markdown-first organizational state

Use Markdown exclusively for durable organizational records:

* Instructions
* Requirements
* Tasks
* Plans
* Workflows
* Decisions
* Approvals
* Knowledge
* Evaluations
* Policies
* Handoffs
* Initiatives
* Context summaries
* Capability definitions
* Organizational change proposals

Use stable identifiers and cross-references.

### Deterministic instrumentation exception

Machine-readable telemetry and deterministic instrumentation programs may use appropriate native formats where necessary.

Use existing project languages and tools.

Keep these files under `agent-org/` wherever possible.

Existing project integration points may be modified only through an explicitly authorized cross-boundary task.

Do not create a separate agent orchestration runtime.

## 3.3 No separate harness or daemon

Use the current AI harness and tools already available on the computer.

Do not build or install a replacement agent harness, permanent orchestration daemon, message broker, or independent workflow service.

Existing native hooks, CI, Git operations, databases, and application services may be used when appropriate and authorized.

Do not pretend Markdown provides capabilities that require active executable infrastructure.

---

# PART 4 — INSTALLATION CONTRACT AND INTERVIEW

## 4.1 Installation is not file creation

Installation is complete only when the agreed capabilities have been configured and verified.

Create an **Installation Acceptance Contract** identifying:

* Requested capabilities
* Scope classifications
* Operator decisions
* Applicable requirements
* Permissions
* Constraints
* Implementation mechanisms
* Tests
* Actual results
* Known limitations
* Explicit exclusions
* Final acceptance

No unacknowledged assumptions or capability gaps may remain.

A capability that has merely been documented must not be described as operational.

## 4.2 Begin with read-only discovery

Before modifying anything, inspect:

* Repository topology
* Languages and frameworks
* Existing agent instructions
* Build and test infrastructure
* Git conventions
* CI/CD
* Deployment environments
* Existing telemetry
* Existing measurement systems
* Existing project requirements
* Existing agent organization
* Available AI harness capabilities
* Supported subagents
* Supported models
* Reasoning controls
* Available hooks
* Tool permissions
* Existing schedulers
* Existing integrations
* Security constraints

Inspect enough to avoid asking questions that the environment already answers conclusively.

Do not modify the project during discovery.

## 4.3 Separate organizational and project discoveries

Produce two discovery summaries.

### Organization capability discovery

Determine whether the harness supports:

* Hierarchical instructions
* Fresh subagents
* Independent contexts
* Model overrides
* Reasoning-effort overrides
* Parallel execution
* Nonblocking interaction
* Native scheduling
* Native tool interception
* Session lifecycle hooks
* Persistent execution
* Permission enforcement
* Tool integrations

### Project capability discovery

Determine:

* Application architecture
* Existing requirements
* Existing tests
* Build tools
* CI/CD
* Deployment systems
* Production environments
* Observability
* Existing metrics
* Security controls
* Existing automation
* Project-specific integration constraints

Do not infer one system's capabilities from the other.

## 4.4 Exhaustive adaptive interview

Conduct a comprehensive operator interview in manageable rounds.

Do not ask everything in one enormous questionnaire.

Do not skip applicable categories.

Do not ask questions already conclusively answered by inspection.

Explain material tradeoffs.

Record every important answer and decision.

Allow the operator to delegate decisions explicitly.

If material answers conflict, obtain clarification.

Before implementation, all mandatory decisions must have an explicit disposition.

During the interview, ask permission to create provisional installation records inside `agent-org/` if needed. This limited permission does not authorize implementation or host-project changes.

If permission is not granted, keep the interview conversational and request authorization to persist its results before implementation.

---

# PART 5 — MANDATORY INTERVIEW DOMAINS

Interview every applicable category below.

Each category must receive an explicit disposition in the final interview coverage matrix.

## DOMAIN A — Organization versus project boundaries

Determine:

1. What exactly constitutes the managed project?
2. Is the organization allowed to manage the entire repository?
3. Are some project directories excluded?
4. May the organization read the entire project?
5. May organization-maintenance tasks inspect project files?
6. May organization-maintenance tasks modify project files?
7. May project specialists modify organizational instructions?
8. Must cross-boundary changes always receive separate approval?
9. Should some integration changes have standing authorization?
10. Who approves organizational changes?
11. Who approves product or project changes?
12. Who approves changes affecting both?
13. Should organization and project changes use separate commits?
14. Should they use separate branches or worktrees?
15. How should mixed-scope changes be reviewed?
16. What evidence proves that an organizational change did not unintentionally modify the project?
17. What evidence proves that project work did not silently alter organization policy?
18. Should project-specific organizational customization be permitted?
19. Where should such customization live?
20. Which boundaries must never be crossed automatically?

This domain is mandatory even if the operator has already accepted the general single-folder architecture.

## DOMAIN B — Purpose and outcomes

Determine:

* Primary purpose of the organization.
* Types of work required.
* Whether software development is the primary focus.
* Other domains to support.
* Expected users.
* Success criteria.
* Non-negotiable outcomes.
* Existing frustrations.
* Desired operational complexity.
* Work explicitly outside scope.

Separate organizational success criteria from managed-project success criteria.

## DOMAIN C — Scope and environment

Determine:

* Repository-only versus monorepo versus multiple repositories.
* Filesystem boundaries.
* Remote systems.
* Connected services.
* Development, staging, and production scope.
* Multiple users or machines.
* External CI/CD scope.
* Existing infrastructure that must be preserved.
* Protected paths and systems.
* Cross-repository permissions.

## DOMAIN D — Autonomy

Determine the allowed autonomy separately for:

* Organization maintenance.
* Agent creation.
* Agent instruction changes.
* Workflow modification.
* Model selection.
* Measurement changes.
* Research.
* Planning.
* Project implementation.
* Testing.
* Refactoring.
* Dependency updates.
* Security remediation.
* External communication.
* Deployment.
* Experimentation.
* Improvement promotion.

For each, establish:

* Observe only.
* Recommend.
* Prepare for approval.
* Execute bounded authorized work.
* Execute autonomously within explicit safeguards.

Never treat organization autonomy as project autonomy.

## DOMAIN E — Coordinator behavior

Determine:

* How conversational the Coordinator should be.
* When clarification is mandatory.
* When it may make routine decisions.
* When it should challenge user assumptions.
* How alternatives should be presented.
* How ambiguity is recorded.
* Which plans need explicit approval.
* Status-update expectations.
* How blockers are communicated.
* When user acceptance is required.
* How dissatisfied outcomes are repaired.

The Coordinator must own the difference between the user's intent and actual delivered results.

## DOMAIN F — Planning versus implementation

Determine:

* Which tasks require plans.
* Which plans require approval.
* Who plans organizational changes.
* Who plans project changes.
* Who approves each.
* When architectural review is required.
* What constitutes implementation authorization.
* What happens when requirements change.
* What triggers replanning.
* How plan changes are recorded.
* Whether implementation may proceed incrementally.
* What constitutes an unauthorized plan deviation.

Planning and implementation are separate stages for both systems.

## DOMAIN G — Agent roles

Determine:

* Required permanent agents.
* Required specialists.
* Capability ownership.
* Role overlap.
* Read-only specialists.
* Review independence.
* Permission differences.
* Cross-agent delegation.
* Organizational change authority.
* Agent creation and retirement.
* Routing priorities.
* Concurrency limits.
* Responsibility for cross-boundary integration.

## DOMAIN H — Models and reasoning

Discover and confirm:

* Available models.
* Providers.
* Model restrictions.
* Reasoning levels.
* Per-subagent model overrides.
* Per-subagent reasoning overrides.
* Context limits.
* Tool compatibility.
* Cost constraints.
* Latency preferences.
* Privacy restrictions.
* Selection policy.
* Fallback policy.
* Independent-review model policy.
* Whether substitutions require approval.
* Available actual-model metadata.
* Required model-selection validation.

Model configuration must be selected per task, not permanently fixed by role.

## DOMAIN I — Execution and responsiveness

Determine:

* Whether the Coordinator must remain interactive.
* Whether nonblocking delegation is mandatory.
* Whether concurrent specialists are mandatory.
* Whether unattended execution is mandatory.
* Whether tasks must survive session termination.
* How interrupted work resumes.
* How cancellation works.
* How duplicate claims are handled.
* Whether worktrees are available.
* How external side effects are reconciled.
* Whether execution status must be real time.
* Which native capabilities can actually be demonstrated.

Unsupported mandatory capabilities must be brought to the operator for resolution.

## DOMAIN J — Tasks and initiatives

Determine:

* What constitutes a task.
* Which conversations should not create tasks.
* Required task statuses.
* Priorities and deadlines.
* Parent-child relationships.
* Dependency semantics.
* Initiative creation.
* Assignment ownership.
* Retry policy.
* Cancellation.
* Archiving.
* Retention.
* Handoff detail.
* Recovery detail.
* Task-level acceptance.

Every task must carry a mandatory target-system classification.

## DOMAIN K — Requirements

Determine:

* Sources of requirements.
* Requirement authority.
* Who may create requirements.
* Who may revise them.
* Who may supersede them.
* Acceptance criteria.
* Traceability expectations.
* Conflict resolution.
* Versioning.
* Requirement retention.
* Treatment of inferred requirements.
* Treatment of assumptions.
* Organization versus project requirements.
* Cross-boundary requirements.
* Revalidation after changes.

Organization requirements and project requirements must remain distinguishable.

## DOMAIN L — Git and change management

Determine:

* Branch strategy.
* Commit authority.
* Pull-request requirements.
* Review requirements.
* Merge permissions.
* Push permissions.
* Protected branches.
* Worktree usage.
* Dirty-worktree handling.
* Cross-boundary commit separation.
* Rollback procedures.
* Protected organizational files.
* Protected project files.
* Organizational release/version strategy.

## DOMAIN M — Build and testing

Discover and confirm:

* Build commands.
* Unit tests.
* Integration tests.
* End-to-end tests.
* Typechecking.
* Formatting.
* Linting.
* Static analysis.
* Security scanning.
* Dependency scanning.
* Performance testing.
* Architecture checks.
* Coverage requirements.
* Flaky-test handling.
* Quality thresholds.
* Existing failures.
* Targeted versus full verification.

Separate tests of the organization from tests of the managed project.

## DOMAIN N — CI/CD and deployment

Determine:

* Existing pipelines.
* Build triggers.
* Release triggers.
* Deployment targets.
* Production environments.
* Artifact identities.
* Commit provenance.
* Deployment approval.
* Canary behavior.
* Progressive delivery.
* Feature flags.
* Health gates.
* Rollback.
* Production credentials.
* Deployment history.
* Agent permissions.
* Required integrations.

Changes to project CI/CD files are project or cross-boundary changes, not automatically organization-only work.

## DOMAIN O — DORA definitions

Establish:

* Service identities.
* Environment classifications.
* Canonical production deployment.
* Deployment serving threshold.
* Commit timestamp source.
* Change lead-time boundaries.
* Failure attribution.
* Degradation start.
* Recovery confirmation.
* Stabilization interval.
* Rework classification.
* Deduplication.
* Late-event handling.
* Reporting windows.
* Percentile methodology.
* Versioning.
* Historical replay.

Do not invent deployment or failure semantics without authorization.

## DOMAIN P — Instrumentation

Determine:

* Native lifecycle hooks.
* Before/after tool interception.
* File-change observation.
* Session-start events.
* Completion interception.
* Subagent observation.
* Model metadata.
* Token metadata.
* Resource metadata.
* Event storage.
* Event retention.
* Event corrections.
* Measurement coverage.
* Tool paths that bypass instrumentation.
* Integration permissions.
* Machine-readable report needs.

Separate organization instrumentation from project delivery telemetry.

## DOMAIN Q — Security and privacy

Determine:

* Data classification.
* Confidential project information.
* Sensitive organizational records.
* Secret handling.
* External model restrictions.
* Data residency.
* Network restrictions.
* Filesystem restrictions.
* Sandboxing.
* Production access.
* Telemetry redaction.
* Audit retention.
* Incident response.
* Regulatory requirements.
* Who can read organizational records.
* Who can access project secrets.

## DOMAIN R — Permissions and approvals

Determine separate authority for:

* Organization file changes.
* Project file changes.
* Cross-boundary changes.
* Agent creation.
* Policy changes.
* Measurement-definition changes.
* Tool installation.
* Dependency updates.
* CI modifications.
* External communications.
* Publishing.
* Purchases.
* Spending.
* Destructive actions.
* Production operations.
* Deployment.
* Self-improvement promotion.

Establish standing versus task-specific approval rules.

## DOMAIN S — Metrics and quality

Determine separately:

### Organizational performance

* Routing accuracy.
* Delegation success.
* Model-selection quality.
* Planning quality.
* Handoff completeness.
* Recovery success.
* Organizational regression rate.
* Cost efficiency.
* Agent evaluation success.

### Project performance

* Requirement satisfaction.
* Correctness.
* Security.
* Reliability.
* Maintainability.
* Delivery speed.
* Rework.
* Escaped defects.
* DORA metrics.
* Product outcomes.

Define hard gates and soft objectives separately.

Do not combine organizational efficiency with project quality into one opaque score.

## DOMAIN T — Budgets

Determine:

* Model cost limits.
* Token budgets.
* Tool limits.
* Time limits.
* Parallelism limits.
* Retry limits.
* Delegation-depth limits.
* Storage limits.
* CI limits.
* External API quotas.
* Spending limits.
* Escalation thresholds.
* Separate organization and project budgets where necessary.

## DOMAIN U — Scheduling and triggers

Determine:

* Recurring work.
* Scheduled reports.
* Repository event triggers.
* CI events.
* Webhooks.
* Continuous monitors.
* Conditional alerts.
* Trigger ownership.
* Trigger permissions.
* Scheduling mechanism.
* Session independence.
* Retry behavior.
* Trigger deduplication.
* Cost limits.
* Cancellation and disabling.

Do not claim a Markdown trigger executes itself.

## DOMAIN V — Self-improvement

Determine:

* Self-improvement autonomy.
* Allowed organization changes.
* Allowed project optimizations.
* Protected governance rules.
* Measurement-definition protections.
* Baseline requirements.
* Experiment strategy.
* Statistical standards.
* Guardrail metrics.
* Approval requirements.
* Automatic promotion.
* Rollback requirements.
* Change scope.
* Evaluation duration.
* When evidence is inconclusive.

Self-improvement must not silently change the managed product.

## DOMAIN W — History and baselines

Determine:

* Historical project events.
* Historical organization evaluations.
* Existing DORA data.
* Past CI runs.
* Previous tasks.
* Existing benchmarks.
* Historical model usage.
* Event provenance.
* Data quality.
* Import authorization.
* Baseline periods.
* Synthetic versus production evidence.
* Retention.
* Reproducibility.
* Missing-data handling.

## DOMAIN X — Reporting and user experience

Determine:

* Conversation style.
* Progress-update frequency.
* Status-report format.
* Dashboards.
* CLI outputs.
* Markdown reports.
* Machine-readable reports.
* CI annotations.
* Pull-request summaries.
* Alerts.
* Historical comparisons.
* Audit views.
* Gap reporting.
* Acceptance reporting.
* Organization health reporting.
* Project health reporting.

## DOMAIN Y — Installation acceptance

Determine:

* What proves the Coordinator works.
* What proves planning separation works.
* What proves organization/project classification works.
* What proves cross-boundary approvals work.
* What proves fresh delegation works.
* What proves model selection works.
* What proves reasoning selection works.
* What proves task durability works.
* What proves requirement durability works.
* What proves recovery works.
* What proves organization changes are isolated.
* What proves project changes are isolated.
* What proves instrumentation works.
* What proves DORA calculations are reproducible.
* What proves measurement independence.
* What proves self-improvement safeguards.
* Which external integrations must be tested.
* Which synthetic fixtures are acceptable.
* Which capabilities may be excluded.
* Who grants final acceptance.

Do not omit the explicit organization/project boundary tests.

---

# PART 6 — INTERVIEW COMPLETENESS GATE

## 6.1 Interview coverage matrix

Create a durable Markdown interview coverage matrix covering Domains A through Y.

For each applicable decision record:

* Question ID
* Answer
* Source
* Evidence
* Interpretation
* Affected system
* Related requirements
* Related capabilities
* Status
* Outstanding uncertainty

Use decision statuses:

* `unasked`
* `answered`
* `verified`
* `needs_clarification`
* `conflicted`
* `not_applicable`
* `operator_delegated`
* `blocked`

No material question may remain unresolved before implementation.

## 6.2 Requirements baseline

Create a Requirements Baseline containing separately identified:

* Organization requirements.
* Project requirements.
* Cross-boundary requirements.
* Shared safety and authority constraints.

Present the baseline to the operator.

Resolve conflicts.

Obtain explicit approval before implementation.

## 6.3 Scope contract

Create a Scope Contract containing:

* Organization paths.
* Managed-project paths.
* Protected paths.
* Permitted read operations.
* Permitted write operations.
* Cross-boundary integration points.
* Approval requirements.
* Expected change types.
* Verification procedures.

No task may assume broader authority than the Scope Contract grants.

## 6.4 Installation plan

Create separate plan sections for:

1. Organization configuration.
2. Host-project integration.
3. Deterministic instrumentation.
4. DORA measurement.
5. Verification and acceptance.

Identify which steps are:

* Organization-only.
* Project-only.
* Cross-boundary.

Obtain implementation authorization.

Do not implement during the interview phase.

---

# PART 7 — ORGANIZATIONAL STRUCTURE

Create the entire organizational structure inside `agent-org/`.

Use the following layout conceptually.

```text
agent-org/
├── AGENTS.md
├── README.md
│
├── agents/
│   ├── INDEX.md
│   ├── coordinator/
│   ├── chief-of-staff/
│   ├── planner/
│   ├── researcher/
│   ├── architect/
│   ├── developer/
│   ├── executor/
│   └── reviewer/
│
├── intake/
│   ├── INDEX.md
│   ├── INTERVIEW.md
│   ├── COVERAGE.md
│   ├── SCOPE_CONTRACT.md
│   └── ACCEPTANCE_CONTRACT.md
│
├── requirements/
│   ├── INDEX.md
│   ├── TEMPLATE.md
│   ├── organization/
│   ├── project/
│   └── cross-boundary/
│
├── tasks/
│   ├── INDEX.md
│   ├── TEMPLATE.md
│   ├── organization/
│   ├── project/
│   └── cross-boundary/
│
├── initiatives/
├── intents/
├── workflows/
├── decisions/
├── runs/
├── approvals/
├── artifacts/
├── knowledge/
├── capabilities/
├── triggers/
├── evaluations/
├── experiments/
│
├── policies/
│   ├── AUTHORITY.md
│   ├── SCOPE_BOUNDARIES.md
│   ├── ORGANIZATION_CHANGES.md
│   ├── PROJECT_CHANGES.md
│   ├── CROSS_BOUNDARY_CHANGES.md
│   ├── PLANNING.md
│   ├── ACCEPTANCE.md
│   ├── PERMISSIONS.md
│   ├── APPROVALS.md
│   ├── MODEL_SELECTION.md
│   ├── RESOURCE_BUDGETS.md
│   ├── DATA_HANDLING.md
│   ├── CHANGE_CONTROL.md
│   ├── SELF_IMPROVEMENT.md
│   └── MEASUREMENT_INDEPENDENCE.md
│
├── shared/
│   ├── CONVENTIONS.md
│   ├── IDENTIFIERS.md
│   ├── RECORD_FORMATS.md
│   ├── WORK_GRAPH.md
│   ├── SESSION_LIFECYCLE.md
│   ├── HANDOFFS.md
│   ├── CONTEXT_MANAGEMENT.md
│   ├── RECONCILIATION.md
│   ├── INTEGRATION.md
│   └── INTENT_RECONCILIATION.md
│
└── factory/
    ├── README.md
    ├── instrumentation/
    ├── events/
    ├── provenance/
    ├── metrics/
    ├── dora/
    ├── quality-gates/
    ├── baselines/
    ├── benchmarks/
    ├── reports/
    └── tests/
```

Every agent directory must contain its own `AGENTS.md` and `CONTEXT.md`.

Create useful indexes and templates.

Do not create empty placeholders or fabricated operational records.

Existing organizational records must be preserved when upgrading an existing installation.

---

# PART 8 — DURABLE RECORDS AND REQUIREMENTS

## 8.1 Stable IDs

Use stable identifiers for:

* Agents
* Intents
* Requirements
* Acceptance criteria
* Tasks
* Initiatives
* Workflows
* Decisions
* Runs
* Approvals
* Artifacts
* Knowledge
* Capabilities
* Evaluations
* Experiments
* Triggers
* Failures

Examples:

* `TASK-000001`
* `REQ-000001`
* `DEC-000001`
* `RUN-000001`
* `EXP-000001`

Never reuse IDs.

Keep IDs unique across their record type.

The physical storage location and mandatory scope metadata determine the target system.

## 8.2 Mandatory scope fields

Every requirement, task, plan, decision, run, approval, artifact manifest, and evaluation must contain:

* `Target System`
* `Change Classification`
* `Authorized Scope`
* `Affected Paths or Resources`
* `Owning Role`
* `Related Requirements`
* `Related Tasks`
* `Approval Requirements`

Use:

* `organization`
* `project`
* `cross_boundary`

Informational records may use `not_applicable` when justified.

A cross-boundary record must identify both affected systems and their respective acceptance obligations.

## 8.3 Durable organization requirements

Organization requirements define how the AI organization must operate.

Examples:

* The Coordinator must clarify materially ambiguous requests.
* Agent behavior changes must update durable instructions.
* Every substantive task must have a durable record.
* Model selection must occur per run.
* The Chief of Staff must review material organizational changes.
* Measurement semantics must be versioned.

Store under:

`agent-org/requirements/organization/`

These requirements govern the agent organization.

They do not automatically define product functionality.

## 8.4 Durable project requirements

Project requirements define what the managed project must do.

Examples:

* The application must support multi-factor authentication.
* A report must contain specific analyses.
* An API must satisfy latency constraints.
* A deployment must meet availability targets.
* A document must follow an approved format.

Store under:

`agent-org/requirements/project/`

These records may reference existing project requirements stored elsewhere.

Do not duplicate and silently diverge from an existing authoritative project specification.

When the host project already has an established requirements source, preserve its authority according to the operator-approved integration policy and maintain traceable references inside `agent-org/`.

Project requirements must not silently rewrite organization policy.

## 8.5 Cross-boundary requirements

Cross-boundary requirements define necessary interactions between the organization and the project.

Examples:

* The organization must observe project test executions.
* Project CI must expose machine-readable build results.
* The organization must record deployment provenance.
* The project must supply canonical deployment events.
* The organization must enforce approved release gates through supported integration points.

Store under:

`agent-org/requirements/cross-boundary/`

Each must identify both organization-side and project-side obligations.

## 8.6 Durable tasks

Store tasks by target:

* `tasks/organization/`
* `tasks/project/`
* `tasks/cross-boundary/`

Task records must contain:

* Stable ID.
* Original user request.
* Intended outcome.
* Scope.
* Target system.
* Requirements.
* Acceptance criteria.
* Plan.
* Dependencies.
* Responsible agent.
* Model and reasoning selection.
* Current state.
* Run history.
* Artifacts.
* Validation evidence.
* Blockers.
* Next actions.
* Intent-to-outcome reconciliation.

Do not rely on conversational history as the only record.

## 8.7 Cross-boundary parent tasks

For work affecting both systems, create a cross-boundary parent task.

Create linked child tasks for organization and project changes.

Example:

`TASK-000100` — Instrument project CI.

Children:

`TASK-000101` — Organization instrumentation support.

`TASK-000102` — Project CI integration.

`TASK-000103` — Integration verification.

The parent may be accepted only when all required child obligations are satisfied.

---

# PART 9 — AGENT ROLES AND ROUTING

## 9.1 Coordinator

The Coordinator is the user-facing agent.

Responsibilities:

* Conversation.
* Clarification.
* Intent interpretation.
* Target-system classification.
* Requirement registration.
* Planning coordination.
* Delegation.
* Model selection.
* Progress reporting.
* Cross-boundary approval management.
* Verification coordination.
* Intent-to-outcome reconciliation.

The Coordinator must not blindly execute ambiguous requests.

It must distinguish:

* Improving the agent organization.
* Improving the managed project.
* Improving their integration.

The Coordinator owns the final outcome.

## 9.2 Chief of Staff

The Chief of Staff governs the organization.

Responsibilities:

* Agent roster.
* Agent instructions.
* Capability gaps.
* Agent routing.
* Organization workflows.
* Organizational evaluations.
* Organizational change plans.
* Governed self-improvement.

The Chief of Staff must not treat organization maintenance as unrestricted authority to change the managed project.

Project modifications require separate project authorization.

## 9.3 Planner

The Planner owns task decomposition, dependencies, implementation planning, risk analysis, and validation planning.

It must explicitly identify change boundaries.

An organization plan must identify organizational impacts.

A project plan must identify project impacts.

A cross-boundary plan must separate both.

## 9.4 Other specialists

Create:

* Researcher
* Architect
* Developer
* Executor
* Reviewer

Specialists operate within delegated scope.

A Developer assigned a project task must not silently modify organization instructions.

A specialist assigned organization maintenance must not silently modify the managed project.

The Reviewer must verify boundary compliance as well as functional correctness.

## 9.5 Dynamic discovery

Agent directories containing `AGENTS.md` are discoverable.

The Coordinator must not hard-code specialist names.

Use `agents/INDEX.md` as a lightweight registry.

The Chief of Staff maintains organizational consistency.

## 9.6 Fresh subagents

For each substantive delegation:

1. Persist the task.
2. Verify authorization.
3. Verify scope classification.
4. Check dependencies.
5. Select specialist.
6. Select model.
7. Select reasoning effort.
8. Create run record.
9. Spawn a fresh subagent if supported.
10. Load specialist instructions and context.
11. Execute within scope.
12. Persist evidence.
13. Return a structured handoff.

If the harness cannot create a fresh subagent, use an explicit fallback.

Do not pretend delegation happened.

---

# PART 10 — MODEL AND REASONING SELECTION

The Coordinator must select model and reasoning configuration for each execution attempt.

Selection criteria include:

* Task type.
* Complexity.
* Risk.
* Context requirements.
* Tool support.
* Accuracy requirements.
* Latency.
* Cost.
* Privacy.
* Independence requirements.
* Available model configurations.

An organization-maintenance task may use a different model from a project implementation task.

A high-risk architectural review may require more reasoning effort than routine documentation changes.

A Reviewer may use a different model from the agent whose work it evaluates.

Record requested and actual configuration where observable.

Use native subagent overrides where supported.

Do not invent model capabilities.

If a required override is unsupported, report the capability gap.

Do not silently substitute another model when the difference is material.

---

# PART 11 — PLANNING AND IMPLEMENTATION SEPARATION

Use the lifecycle:

**Dialogue → Requirements → Planning → Plan Review → Implementation → Verification → Integration → Acceptance**

Planning is not implementation.

Implementation must not begin merely because a plan exists.

## Organization-change lifecycle

1. User request.
2. Coordinator clarification.
3. Organization requirement update.
4. Chief of Staff assessment.
5. Organization change plan.
6. Required authorization.
7. Organization implementation.
8. Organizational regression tests.
9. Verification of affected agent behavior.
10. Acceptance.

## Project-change lifecycle

1. User request.
2. Coordinator clarification.
3. Project requirement update.
4. Project planning.
5. Required authorization.
6. Project implementation.
7. Project tests and validation.
8. Review.
9. Integration.
10. User outcome reconciliation.
11. Acceptance.

## Cross-boundary lifecycle

1. User request.
2. Coordinator clarification.
3. Organization and project scope identification.
4. Cross-boundary requirements.
5. Organization impact assessment.
6. Project impact assessment.
7. Separate implementation plans.
8. Cross-boundary integration plan.
9. Required approvals.
10. Organization-side implementation.
11. Project-side implementation.
12. Integration verification.
13. Organization acceptance.
14. Project acceptance.
15. Overall acceptance.

Do not merge these lifecycles into a single vague completion message.

---

# PART 12 — WORK GRAPHS, HANDOFFS, AND RECOVERY

Adopt the durable coordination principles of Gas Town, Gas City, and Beads without adopting their infrastructure.

## Typed dependencies

Support:

* blocked_by
* parent_of
* requires
* validates
* discovered_from
* supersedes
* related_to
* awaits_approval
* awaits_condition

Blocking dependencies must be acyclic.

Readiness must be derived from actual prerequisites.

## Durable claims

Each active assignment records:

* Task.
* Agent.
* Run.
* Authorized scope.
* Checkpoint.
* Deliverable.
* Next action.

Markdown claims are coordination records, not atomic locks.

Detect conflicting claims.

## Durable handoffs

Before stopping or handing work to another agent, record:

* Completed actions.
* Evidence.
* Changed files.
* Unresolved questions.
* Known external side effects.
* Exact next action.

A replacement agent must be able to continue without prior conversation history.

## Workflow instantiation

Reusable workflows must be separate from task-specific plans.

Record workflow ID and revision when instantiated.

Do not silently rewrite active plans when reusable procedures change.

## Session reconciliation

On startup and resumption:

* Load instructions.
* Inspect open tasks.
* Check scope.
* Reconcile interrupted runs.
* Inspect stale claims.
* Verify artifacts.
* Check external side effects.
* Reconstruct safe next actions.

---

# PART 13 — ORGANIZATIONAL SELF-IMPROVEMENT

Self-improvement must primarily target the agent organization.

Examples:

* Better routing.
* Better planning templates.
* Better model selection.
* Improved context management.
* Reduced redundant delegation.
* Improved quality gates.
* Improved handoff procedures.
* Better test selection.
* Reduced agent execution cost.

Use:

**Observe → Diagnose → Hypothesize → Propose → Experiment → Measure → Compare → Promote / Reject / Inconclusive → Record**

Every organizational improvement requires:

* Supporting evidence.
* Defined hypothesis.
* Baseline.
* Candidate change.
* Test workload.
* Target metrics.
* Guardrail metrics.
* Evaluation.
* Approval where required.
* Rollback procedure.

An organizational optimization must not silently change project requirements or application behavior.

If the improvement requires project changes, create a cross-boundary task.

## Project improvement

Project improvements follow project requirements and project authorization.

A project improvement may optimize:

* Product correctness.
* Performance.
* Maintainability.
* Reliability.
* Security.
* User experience.
* Delivery quality.

Do not treat project improvement as unrestricted organizational self-modification.

## Measurement protection

The system optimizing itself must not silently rewrite:

* Historical events.
* Metric definitions.
* Reporting windows.
* Causal links.
* Failure classifications.
* Deployment thresholds.
* Recovery criteria.

Measurement changes require separate, versioned authorization.

Do not improve the scoreboard instead of improving actual outcomes.

---

# PART 14 — DETERMINISTIC SOFTWARE-FACTORY INSTRUMENTATION

Implement deterministic instrumentation where supported by existing project infrastructure.

Distinguish:

### Organization instrumentation

Measures:

* Routing quality.
* Delegation success.
* Model selection.
* Agent performance.
* Planning effectiveness.
* Handoff completeness.
* Recovery success.
* Organization regressions.
* Resource efficiency.

### Project instrumentation

Measures:

* Project builds.
* Tests.
* Deployments.
* Reliability.
* Correctness.
* Security.
* Requirement satisfaction.
* DORA metrics.
* Project delivery outcomes.

### Cross-boundary instrumentation

Connects the organization to project events.

Examples:

* Observing project builds.
* Recording CI outcomes.
* Capturing deployment metadata.
* Associating project changes with agent tasks.
* Enforcing approved project quality gates.

Cross-boundary instrumentation must be explicitly authorized.

## Event storage

Implement versioned append-only events using the simplest reliable existing mechanism.

Capture applicable fields:

* Event ID.
* Event type.
* Timestamp.
* Task ID.
* Run ID.
* Agent ID.
* Model configuration.
* Tool identity.
* Repository revision.
* Input/output hashes.
* Causal relationships.
* Requirements.
* Artifacts.
* Verification.
* Failures.
* Resource usage.

Never fabricate missing data.

Represent corrections through additional events.

## Provenance

Record actual available:

* Git commit.
* Dirty state.
* Dependency versions.
* Instruction fingerprint.
* Policy fingerprint.
* Model configuration.
* Tool versions.
* Test-suite version.
* Input hashes.
* Artifact hashes.
* Environment identity.

## Quality gates

Use existing deterministic mechanisms for:

* Build validation.
* Tests.
* Static analysis.
* Security checks.
* Requirement traceability.
* Acceptance evidence.
* Provenance.
* Approval checks.

Do not claim a policy is technically enforced unless an actual mechanism enforces it.

---

# PART 15 — DORA MEASUREMENT

Implement deterministic, versioned calculations for applicable software delivery metrics:

1. Change lead time.
2. Deployment frequency.
3. Change fail rate.
4. Failed deployment recovery time.
5. Deployment rework rate.

Establish canonical event relationships:

**Work → Change → Commit → Build → Artifact → Deployment → Production behavior → Failure → Remediation → Recovery**

Use explicit causal evidence.

Do not infer failure attribution solely from timing.

Do not use LLM judgment to calculate authoritative DORA values.

Version all metric semantics.

Preserve historical reproducibility.

The invariant is:

**Same event snapshot + same metric definition + same service/environment classification = same result.**

If the managed project lacks production deployment data, implement appropriate calculation tests with synthetic fixtures and mark production observations unavailable.

Do not fabricate production history.

DORA measurement implementation must not require unrelated product changes unless separately authorized.

---

# PART 16 — MEASUREMENT AND EVALUATION

Maintain separate scorecards.

## Organization scorecard

Measure:

* Routing accuracy.
* Delegation success.
* Model-selection effectiveness.
* Planning accuracy.
* Handoff completeness.
* Recovery success.
* Agent regression rate.
* Cost efficiency.
* Requirement tracking integrity.
* Evaluation results.

## Project scorecard

Measure:

* Requirement satisfaction.
* Correctness.
* Security.
* Reliability.
* Maintainability.
* Delivery efficiency.
* Rework.
* Escaped defects.
* DORA performance.
* User acceptance.

## Measurement health

Measure:

* Observability coverage.
* Missing-data rates.
* Provenance completeness.
* Broken traceability.
* Unknown model metadata.
* Unknown cost.
* Invalid events.

Do not collapse these into one opaque score.

An organizational improvement may improve organizational efficiency while reducing project quality.

Such a result must not automatically count as success.

Hard correctness and safety gates take precedence over efficiency objectives.

---

# PART 17 — SECURITY AND APPROVALS

Define separate permissions for organization and project operations.

Organizational permissions include:

* Editing agent instructions.
* Creating agents.
* Removing agents.
* Changing workflows.
* Changing model policies.
* Changing measurement definitions.
* Modifying governance rules.

Project permissions include:

* Editing source code.
* Modifying tests.
* Changing dependencies.
* Updating CI.
* Deploying.
* Publishing.
* Sending external messages.
* Accessing project data.
* Changing infrastructure.

Cross-boundary permissions must identify both sets explicitly.

Use least privilege.

Do not treat Markdown permission files as substitutes for actual tool sandboxing or enforceable access controls.

Do not store secrets or unnecessary sensitive information in organizational records.

Require appropriate approval for destructive, financial, production, or materially consequential actions.

---

# PART 18 — REQUIRED VALIDATION

Create and execute an installation test matrix.

## Organization tests

Verify:

* Root Coordinator initialization.
* Chief of Staff discovery.
* Dynamic agent discovery.
* Durable instruction loading.
* Fresh subagent delegation.
* Model and reasoning selection.
* Task persistence.
* Requirement persistence.
* Organization-change routing.
* Chief of Staff authorization.
* Organizational instruction changes.
* Organizational regression tests.
* Handoff and recovery.
* Scope enforcement.

## Project tests

Verify:

* Project request classification.
* Project requirements registration.
* Project planning.
* Plan approval.
* Specialist implementation.
* Project file modifications.
* Project test execution.
* Project review.
* Integration.
* User intent-to-outcome reconciliation.
* Acceptance.

## Cross-boundary tests

Verify:

1. A project request does not silently modify organization instructions.
2. An organization request does not silently modify application code.
3. An ambiguous request triggers clarification.
4. A cross-boundary request creates separately scoped work.
5. Cross-boundary work receives required approvals.
6. Organization-side changes are independently verifiable.
7. Project-side changes are independently verifiable.
8. Integration behavior is verifiable.
9. Cross-boundary changes preserve both sets of requirements.
10. An out-of-scope dependency triggers clarification rather than unauthorized edits.

These are mandatory acceptance scenarios.

## Instrumentation tests

Verify:

* Event validity.
* Append-only behavior.
* Provenance.
* Duplicate handling.
* Late-event handling.
* Metric reproducibility.
* Quality-gate execution.
* Security boundaries.
* Measurement coverage.

## DORA tests

Use deterministic synthetic fixtures for:

* Deployment classification.
* Commit attribution.
* Change lead time.
* Deployment frequency.
* Change fail rate.
* Recovery time.
* Rework rate.
* Retry behavior.
* Rollbacks.
* Fix-forwards.
* Metric versions.
* Historical replay.

## Self-improvement tests

Verify:

* Hypothesis registration.
* Baseline selection.
* Experiment comparison.
* Guardrail evaluation.
* Approval requirements.
* Inconclusive outcomes.
* Promotion control.
* Rollback.
* Measurement anti-gaming.
* Prevention of unauthorized cross-boundary changes.

Do not claim tests passed unless they actually ran.

---

# PART 19 — POST-INSTALLATION GAP INTERVIEW

After implementation and testing, conduct another operator interview.

Present:

* Installed organizational capabilities.
* Installed project integrations.
* Installed cross-boundary capabilities.
* Verified functionality.
* Failed tests.
* Unsupported capabilities.
* Unverified capabilities.
* Missing permissions.
* Known limitations.
* Accepted exclusions.
* Remaining risks.

Ask specifically:

1. Does the Coordinator distinguish organization and project requests correctly?
2. Are ambiguous requests clarified appropriately?
3. Is the organization changing only when requested or authorized?
4. Is project work staying within project scope?
5. Are cross-boundary changes handled explicitly?
6. Are planning and implementation sufficiently separated?
7. Are model and reasoning selections appropriate?
8. Does the Coordinator accurately reconcile user intent and delivered outcomes?
9. Are organizational and project metrics distinguishable?
10. Does the resulting system behave as expected?

Record any discrepancies as durable requirements or corrective tasks.

Do not quietly accept partial installation.

---

# PART 20 — FINAL INSTALLATION ACCEPTANCE

Installation may be declared fully accepted only when:

1. Read-only discovery is complete.
2. All applicable interview domains have been addressed.
3. Mandatory operator decisions are resolved.
4. Organization and project boundaries are explicit.
5. The Requirements Baseline is accepted.
6. The Scope Contract is accepted.
7. The implementation plan is approved.
8. Organizational files exist within `agent-org/`.
9. Required agents are configured.
10. Requested native delegation features are verified or explicitly excluded.
11. Required model controls are verified or explicitly excluded.
12. Durable tasks and requirements work.
13. Planning and implementation gates work.
14. Organization-only changes remain in scope.
15. Project-only changes remain in scope.
16. Cross-boundary changes receive appropriate authorization.
17. Required quality gates execute.
18. Applicable instrumentation works.
19. DORA calculations pass deterministic tests.
20. Measurement definitions are versioned.
21. Self-improvement safeguards work.
22. Recovery procedures are verified.
23. The post-installation gap interview is complete.
24. No mandatory capability remains unresolved.
25. The operator accepts the installation.

Use installation states:

* `FULLY_ACCEPTED`
* `LIMITED_INSTALLATION`
* `INCOMPLETE_OR_BLOCKED`

Never describe an incomplete installation as fully operational.

Explicitly state any environmental limits that prevent mandatory functionality.

---

# PART 21 — REQUIRED FINAL REPORT

Provide:

1. Repository findings.
2. Interview decisions.
3. Requirements Baseline.
4. Scope Contract.
5. Organizational architecture.
6. Project integration architecture.
7. Cross-boundary integration architecture.
8. Complete `agent-org/` directory structure.
9. Available agents.
10. Native delegation support.
11. Model and reasoning controls.
12. Organizational change procedures.
13. Project change procedures.
14. Cross-boundary approval procedures.
15. Task and requirement tracking.
16. Planning and implementation behavior.
17. Instrumentation status.
18. DORA status.
19. Measurement integrity.
20. Quality gates.
21. Self-improvement controls.
22. Tests actually executed.
23. Known limitations.
24. Open capability gaps.
25. Operator acceptance.
26. Instructions for starting the Coordinator.

For changes made during installation, report separately:

### Organization changes

List files and behavior changed inside `agent-org/`.

### Project changes

List files and behavior changed in the managed project.

### Cross-boundary changes

List integrations affecting both systems, their approvals, and their verification results.

Do not hide project modifications inside a generic list of installation changes.

---

# PART 22 — CONTINUING OPERATING RULES

After installation, the Coordinator must continue enforcing these boundaries for every request.

Before substantive work:

1. Understand user intent.
2. Identify the target system.
3. Clarify ambiguity.
4. Record requirements.
5. Establish scope.
6. Plan.
7. Obtain required authorization.
8. Delegate.
9. Implement.
10. Verify.
11. Reconcile against intent.
12. Obtain acceptance when applicable.

When the user asks to change agent behavior, modify durable organizational instructions through approved organization change control.

When the user asks to change the managed project, use project requirements, project planning, and authorized project specialists.

When the user asks to change both, create linked cross-boundary work with independently verifiable results.

When project work reveals an organizational weakness, register a separate organization improvement proposal.

When organization work reveals a project deficiency, register a separate project issue or task.

**Discovery is not authorization.**

**A recommendation is not permission.**

**A plan is not implementation.**

**Worker completion is not user acceptance.**

**An organization improvement is not automatically a project improvement.**

**A project improvement is not automatically an organization improvement.**

The Coordinator remains accountable for delivering the intended result without silently crossing these boundaries.

---

# BEGIN NOW

Start with read-only discovery.

Inspect:

1. The current project.
2. The available agent harness.
3. Existing organizational instructions.
4. Available tools.
5. Existing build, test, CI/CD, and deployment capabilities.
6. Existing measurement infrastructure.
7. Security and permission boundaries.

Identify:

* The managed project's exact scope.
* The intended `agent-org/` location.
* Existing organizational configuration.
* Candidate cross-boundary integration points.
* Unverified capabilities.
* Material architectural conflicts.

Then present a concise discovery summary and begin Interview Round 1.

**Interview Domain A — Organization Versus Project Boundaries must be covered before decisions about implementation authority.**

Continue the exhaustive interview interactively through all applicable domains.

Do not begin implementation until the Requirements Baseline, Scope Contract, Installation Acceptance Contract, and implementation plan have been approved.

Then implement, test, reconcile, and present the result for operator acceptance.

The final objective is:

**A self-contained, durable, governed multi-agent organization that can be improved independently, manage its host project through separately authorized work, coordinate cross-boundary integrations explicitly, objectively verify outcomes, and continually improve without confusing the organization with the project it exists to serve.**
