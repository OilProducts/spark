---
type: Decision
title: Spark is the first organization
id: ADR-0008
status: Accepted
date: 2026-10-28
---

# ADR-0008: Spark is the first organization

## Status

Accepted

## Context

The operator wants Spark to be self-consistent. Spark's own development already
runs through Spark: a Coordinator, missions, and change requests implemented by
flows. But its records follow none of the conventions this seed adopts:

- the repository has no changelog and no versioned releases;
- design decisions live inside change requests (local and gitignored) and in an
  assistant's private memory notes.

## Decision

Spark's own development is the first organization built from this seed, with
the core and the `software` profile.

When the seed is accepted, two things follow:

- **The Spark project's org repo** is set up from the seed. That repo is
  `<spark>/.spark/agent-org/`.
- **Spark's repository** adopts the software profile's conventions as project
  work: a `CHANGELOG.md`, Semantic Versioning for releases, and the project's
  architecture decisions as decision records in the repository (SW-003). That
  includes the decisions currently held only in change requests and memory
  notes.

## Consequences

- Every gap in the seed shows up first in Spark's own development.
- Migrating the existing decisions is real work. It is project work for Spark,
  done through flows, not by hand.
- The operator brief for Spark's organization starts from the operator's quotes
  already recorded in these decision records.

## Source

Operator, 2026-10-28. Asked "How far should Spark self-consistency go?", chose
"Spark is the first org", described as: "Spark's own development runs as an org
from this seed; Spark's repo adopts CHANGELOG, SemVer releases and ADRs
(replacing decisions in CRs and memory)." Context from the operator,
2026-10-28: "I would like to keep spark self-consistent."
