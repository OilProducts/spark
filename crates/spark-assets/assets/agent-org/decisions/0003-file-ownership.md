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

Partly superseded, 2026-11-01: initialization now copies the whole seed spec,
every profile as reference material, and every bundled flow, role and playbook
into `<project>/.spark/definitions/` when the project is registered. Ownership
and update rules are unchanged.

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
- **The organization owns** everything that defines how it works: its spec (the
  core plus its chosen profiles), its records, its README and changelog, its
  standing roles, and the flows, roles and playbooks it runs. Initialization
  (ADR-0004) copies the spec from the seed, and copies from Spark's bundled
  library the standing roles, the `organization/` flows and each chosen
  profile's flows, with the roles those flows use. The `software` profile brings
  `software-development/`; the other profiles bring no flows yet. Spark never
  changes the organization's files afterwards.
- **Spark's bundled library is a source, not a fallback.** An organization runs
  only what it holds (ORG-025). To use another bundled flow, role or playbook,
  the Chief of Staff copies it in first.
- **Before initialization** a project has no organization, and its standing
  roles run from Spark's bundled copies.

The Chief of Staff owns the standing roles and is responsible for the
organization's health. Updates from Spark reach an organization only when the
operator asks for them: the Chief of Staff compares Spark's seed and bundled
library against the versions the organization recorded (ORG-035), and adopts
what fits.

## Consequences

- Orgs are stable: a Spark update cannot change an org's spec, roles or flows.
- Fixes to Spark's seed, roles and flows do not reach existing orgs until the
  operator asks for them.
- Initialization copies more files, and using a bundled flow the org has not
  copied takes a copy first.
- Spark needs an installation step that copies the seed and the bundled files
  (ADR-0004).

## Source

Proposed by the agent drafting the seed, 2026-10-28, building on the operator's
2026-10-28 description of independent evolution (quoted under ORG-040).
Accepted by the operator, 2026-10-28: "I approve ADR-0003, ADR-0004, and
ADR-0005 as written." Revised before release at the operator's direction,
2026-10-29: "The CoS owns standing roles and is responsible for the health of
the agent org. Once the org is initialized it is independent from the bundled
org, but initially thats where they are, bundled. After that, updates from
spark to an already initialized org are explicit, managed by the CoS and
initialized by request of the user." Asked whether that independence also
covers flows and worker roles, the operator chose that it does, 2026-10-29:
"#8. remaining gap, I choose A."
