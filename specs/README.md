# Specs

This directory contains the canonical target-state documents for Spark Spawn, plus the referenced Attractor source specification.

These files define the program we are building:
- core product and runtime specifications
- user stories and detailed story records
- selected target-behavior adjuncts such as boundaries

Execution artifacts stay outside this folder:
- checklists
- migration plans
- risk reports
- gap inventories
- implementation notes

## Canonical Documents

- `attractor-spec.md`
  Core runtime and DSL specification.
- `sparkspawn-attractor-extensions.md`
  Spark Spawn-specific extensions to the core Attractor runtime.
- `ui-spec.md`
  Canonical web UI specification.
- `ui-user-stories.md`
  Stable user-story catalog.
- `ui-story-records.md`
  Detailed story records with rationale, acceptance criteria, non-goals, and implementation intent.
- `conversation-paradigm.md`
  Canonical definition of project chat conversations, turns, streaming, and review artifacts.
- `storage-boundaries.md`
  Canonical definition of app-owned versus project-owned data and where each must live.

## Supporting Spec Documents

- `ui-runtime-parser-boundaries.md`

## Acceptance Workflow Assets

High-level workflow verification assets live outside `specs/` under:
- `/Users/chris/tinker/sparkspawn/tests/acceptance/agent-workflows/`
