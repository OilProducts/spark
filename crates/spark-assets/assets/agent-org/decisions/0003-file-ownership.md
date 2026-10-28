---
type: Decision
title: File ownership after installation
id: ADR-0003
status: Accepted
date: 2026-10-28
---

# ADR-0003: File ownership after installation

## Status

Accepted

## Context

Spark resolves flows and roles project first, then from Spark's bundled library.
As a result, every Spark update changes every org that has not overridden a
file. For example, CR-2026-0169 rewrote the bundled `coordinator.md`, which
changed every project's Coordinator at once.

Each org is meant to evolve independently (spec ORG-040), which conflicts with
that. Two other tools handle this by giving every file exactly one owner:

- **factoryctl**: initialization copies the starter in, and the project owns it
  from then on.
- **OpenSpec**: tool-generated files belong to the tool; user configuration is
  seeded once and never touched again; schemas resolve in layers and are forked
  to customize.

## Decision

Every file has exactly one owner.

- **Spark owns** the runtime and its fixed instructions. Spark updates them
  freely, and orgs never edit them.
- **The organization owns** its spec (the core plus its chosen profiles), its
  standing roles (Coordinator and Chief of Staff), its records, its README and
  its changelog. They are copied from the seed at
  installation, and Spark never changes them afterwards.
- **Layered**: generic flows and worker roles resolve from Spark's bundled
  library until the org copies one to own it.

Seed improvements reach an org only when its Chief of Staff reviews what changed
in the seed since the version it recorded (ORG-035) and adopts what fits.

## Consequences

- Orgs are stable: a Spark update cannot change an org's Coordinator or Chief of
  Staff.
- Fixes to the seed's roles do not reach existing orgs on their own.
- Spark needs an installation step that copies the seed (ADR-0004).

## Source

Proposed by the agent drafting the seed, 2026-10-28, building on the operator's
2026-10-28 description of independent evolution (quoted under ORG-040).
Accepted by the operator, 2026-10-28: "I approve ADR-0003, ADR-0004, and
ADR-0005 as written."
