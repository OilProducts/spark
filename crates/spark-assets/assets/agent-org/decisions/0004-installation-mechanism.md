---
type: Decision
title: How installation runs
id: ADR-0004
status: Accepted
date: 2026-10-28
---

# ADR-0004: How installation runs

## Status

Accepted

## Context

Setting up an organization has two different kinds of step:

- **Mechanical:** create the org repo as a git repository, copy the core and the
  chosen profiles, and write the first README and changelog entry.
- **Judgment:** interview the operator, shape the spec, and add roles.

Today nothing creates the org repo. It appears only when the flow editor first
saves a project flow. factoryctl does the mechanical part with a deterministic
`init` command.

## Decision

- **A deterministic `spark org init --project <path> --profile <name>...`
  command** does the mechanical part. It never overwrites an existing file and
  reports what it created.
- **The setup playbook** then runs as a mission to do the judgment part.

## Consequences

- Installation becomes reproducible and testable without a model.
- Requires a Spark change: the command, plus the seed embedded in the build.

## Source

Proposed by the agent drafting the seed, 2026-10-28. Accepted by the operator,
2026-10-28: "I approve ADR-0003, ADR-0004, and ADR-0005 as written."
