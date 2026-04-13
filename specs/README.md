# Specs

This directory contains the canonical target-state documents for Spark, plus the referenced Attractor source specification.

These files define the program we are building:
- core product and runtime specifications
- client and workspace behavior contracts
- selected target-behavior adjuncts

Execution artifacts stay outside this folder:
- checklists
- migration plans
- risk reports
- gap inventories
- implementation notes

## Canonical Documents

- `attractor-spec.md`
  Core runtime and DSL specification.
- `spark-workspace.md`
  Canonical workspace-layer specification above Attractor: projects, conversations, review artifacts, approvals, provenance, storage boundaries, and workspace service behavior.
- `spark-ui-ux.md`
  Canonical operator-facing client specification: presentation boundaries, workflows, UX rules, client-state rules, and story traceability.
- `spark-flow-extensions.md`
  Canonical Spark-owned flow-surface extension contract layered onto Attractor.

## Acceptance Workflow Assets

High-level workflow verification assets live outside `specs/` under:
- `../tests/acceptance/agent-workflows/`
