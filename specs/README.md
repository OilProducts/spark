# Specs

This directory contains the canonical target-state documents for Spark Spawn, plus the referenced Attractor source specification.

These files define the program we are building:
- core product and runtime specifications
- user stories and detailed story records
- selected target-behavior adjuncts such as boundaries, scenarios, and acceptance scripts

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

- `ui-role-persona-scenarios.md`
- `ui-parity-complete-user-journey.md`
- `ui-runtime-parser-boundaries.md`
