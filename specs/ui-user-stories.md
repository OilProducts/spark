# Spark Spawn UI User Stories: Home-First Project Workflow

This document captures user stories implied by the current Spark Spawn product direction and the `project` concept.

A **project** is a user-selected work target with these invariants:
- It is identified by a unique directory path.
- It must be backed by a Git repository.
- It scopes conversation context, specs, plans, work items, runs, and artifacts.

---

## 1. Home Workspace and Project Selection

- **US-HOME-01**
  As a user, I want a top-level `Home` tab that combines project selection and AI collaboration so that I can start work from one place.

- **US-HOME-02**
  As a user, I want a left sidebar in Home with my projects so that switching projects is a single click.

- **US-HOME-03**
  As a user, I want Home to clearly show the active project identity (name, directory, branch) so that I always know which repo I am operating on.

- **US-PROJ-01**
  As a user, I want to create or register a project from a local directory so that all work is anchored to a concrete filesystem location.

- **US-PROJ-02**
  As a user, I want the UI to prevent duplicate projects pointing to the same directory so that project identity stays unambiguous.

- **US-PROJ-03**
  As a user, I want the UI to verify the selected directory is a Git repository (or guide me to initialize one) so that project workflows always run with version-control context.

- **US-PROJ-04**
  As a user, I want to pick one active project and see that selection clearly in global navigation so that I always know which repo I am operating on.

- **US-PROJ-05**
  As a user, I want recent/favorite project switching in the Home sidebar so that I can move between efforts without re-entering paths.

- **US-PROJ-06**
  As a user, I want project metadata (name, directory, current branch, last activity) visible at a glance so that I can choose the right project confidently.

---

## 2. Project-Scoped AI Conversation and Spec Editing

- **US-CONV-01**
  As a project author, I want to open a project-scoped conversation with an AI agent in Home so that I can define requirements in context of that project.

- **US-CONV-02**
  As a project author, I want conversation context to include project directory and repository state so that AI suggestions align with actual project files and structure.

- **US-CONV-03**
  As a project author, I want to iteratively draft and refine a specification with the AI (like this chat workflow) so that spec authoring is collaborative and traceable.

- **US-CONV-04**
  As a project author, I want AI-proposed spec edits to be explicit and reviewable before apply so that spec changes are intentional.

- **US-CONV-05**
  As a project author, I want conversation history saved per project so that decisions and rationale remain discoverable later.

- **US-CONV-06**
  As a user, I want strict isolation between project conversations so that context and files from one project never leak into another.

- **US-CONV-07**
  As a user, I want to close and reopen the app without losing a project conversation so that I can resume the same AI thread later.

- **US-CONV-08**
  As a user, I want to start a new conversation thread within the active project so that I can explore a new line of work without mixing it into the existing thread.

- **US-CONV-09**
  As a user, I want to see the list of conversation threads for the active project so that I can switch between prior discussions and recover the right context quickly.

- **US-CONV-10**
  As a user, I want each conversation thread to preserve its own history, artifacts, and underlying AI session so that returning to a thread resumes the correct context.

- **US-CONV-11**
  As a user, I want new threads to start empty but remain scoped to the same active project directory and repository so that thread isolation does not break project isolation.

- **US-CONV-12**
  As a user, when I send a message in project chat, I want to see the assistant reply stream into the conversation as it is generated so that I can tell the model is actively responding and follow the answer before it completes.

  Detailed record: `ui-story-records.md#us-conv-12`

---

## 3. Spec -> Plan -> Work Tracker -> Build Chain

- **US-WORK-01**
  As a project author, I want accepted spec edits to trigger a DOT orchestration that generates an implementation plan so that planning is automatic and repeatable.

- **US-WORK-02**
  As a project author, I want that orchestration to produce plan artifacts and candidate work items so that planning outputs are actionable.

- **US-WORK-03**
  As a reviewer/operator, I want a human approval gate before work items are published to the tracker so that implementation does not start without explicit approval.

- **US-WORK-04**
  As an operator, I want approved work items to transition to `ready` in the tracker so that implementation agents can pick them up.

- **US-WORK-05**
  As an operator, I want to launch build/implementation workflows from approved ready work so that execution is tied to governed scope.

- **US-WORK-06**
  As an operator, I want live run status, logs, and artifacts for planning and build orchestrations so that I can monitor progress and troubleshoot failures.

- **US-WORK-07**
  As a project author, I want failed workflow runs to produce actionable diagnostics and rerun options so that I can recover quickly.

---

## 4. Governance, Safety, and Auditability

- **US-GOV-01**
  As a user, I want workflow start to be blocked when no active project is selected so that actions cannot run without explicit project scope.

- **US-GOV-02**
  As a user, I want workflow start to be blocked (or explicitly warned) when project Git state violates policy so that risky execution is visible.

- **US-GOV-03**
  As an auditor, I want each spec/plan/tracker/build run linked to project, commit/branch context, and timestamps so that outcomes are traceable.

- **US-GOV-04**
  As a user, I want durable run history per project so that I can inspect past specs, plans, work-item approvals, artifacts, and decisions.

- **US-GOV-05**
  As a user, I want non-destructive failure handling (no silent file loss) when workflows or saves fail so that project state remains trustworthy.

---

## 5. UX and Information Architecture Implications

- **US-IA-01**
  As a user, I want Home to be the default top-level workspace for project selection and conversation so that the core loop starts in one obvious place.

- **US-IA-02**
  As a user, I want deep-linkable state for `project + conversation + run` so that I can share/reopen exact working context.

- **US-IA-03**
  As a user, I want consistent navigation between Home conversation, spec editing, workflow execution, and run inspection so that the end-to-end loop feels unified.
