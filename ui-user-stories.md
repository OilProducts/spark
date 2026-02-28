# Attractor UI User Stories: Project-Centric Workflow

This document captures user stories implied by the current Attractor direction and the new `project` concept.

A **project** is a user-selected work target with these invariants:
- It is identified by a unique directory path.
- It must be backed by a Git repository.
- It scopes conversation context, specs, plans, runs, and artifacts.

---

## 1. Project Identity and Selection

- **US-PROJ-01**
  As a user, I want to create or register a project from a local directory so that all work is anchored to a concrete filesystem location.

- **US-PROJ-02**
  As a user, I want the UI to prevent duplicate projects pointing to the same directory so that project identity stays unambiguous.

- **US-PROJ-03**
  As a user, I want the UI to verify the selected directory is a Git repository (or guide me to initialize one) so that project workflows always run with version-control context.

- **US-PROJ-04**
  As a user, I want to pick one active project and see that selection clearly in global navigation so that I always know which repo I am operating on.

- **US-PROJ-05**
  As a user, I want recent/favorite project switching so that I can move between efforts without re-entering paths.

- **US-PROJ-06**
  As a user, I want project metadata (name, directory, current branch, last activity) visible at a glance so that I can choose the right project confidently.

---

## 2. Project-Scoped AI Conversation

- **US-CONV-01**
  As a project author, I want to open a project-scoped conversation with an AI agent so that I can define requirements in context of that project.

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

---

## 3. Spec -> Plan -> Build Workflow Chain

- **US-WORK-01**
  As a project author, I want to run a workflow that converts the approved project spec into an implementation plan so that planning is standardized and repeatable.

- **US-WORK-02**
  As a project author, I want the generated implementation plan written to a project file with clear status and provenance so that it can be reviewed and versioned.

- **US-WORK-03**
  As a reviewer/operator, I want to approve, reject, or request revision of a generated plan so that build execution is gated by human intent.

- **US-WORK-04**
  As an operator, I want to launch build workflows from the approved plan so that implementation execution is directly tied to agreed scope.

- **US-WORK-05**
  As an operator, I want live run status, logs, and artifacts for planning/build workflows so that I can monitor progress and troubleshoot failures.

- **US-WORK-06**
  As a project author, I want failed workflow runs to produce actionable diagnostics and rerun options so that I can recover quickly.

---

## 4. Governance, Safety, and Auditability

- **US-GOV-01**
  As a user, I want workflow start to be blocked when no active project is selected so that actions cannot run without explicit project scope.

- **US-GOV-02**
  As a user, I want workflow start to be blocked (or explicitly warned) when project Git state violates policy so that risky execution is visible.

- **US-GOV-03**
  As an auditor, I want each spec/plan/build run linked to project, commit/branch context, and timestamps so that outcomes are traceable.

- **US-GOV-04**
  As a user, I want durable run history per project so that I can inspect past specs, plans, artifacts, and decisions.

- **US-GOV-05**
  As a user, I want non-destructive failure handling (no silent file loss) when workflows or saves fail so that project state remains trustworthy.

---

## 5. UX and Information Architecture Implications

- **US-IA-01**
  As a user, I want Projects to be a first-class top-level area in the UI so that selecting and managing project scope is explicit.

- **US-IA-02**
  As a user, I want deep-linkable state for `project + conversation + run` so that I can share/reopen exact working context.

- **US-IA-03**
  As a user, I want consistent navigation between project context, spec editing, workflow execution, and run inspection so that the end-to-end loop feels unified.

