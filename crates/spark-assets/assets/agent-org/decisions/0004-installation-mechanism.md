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

Superseded, 2026-11-01: installation is automatic. Registering a project
copies the whole seed, every profile and every bundled flow, role and playbook
into `<project>/.spark/definitions/` once, without Git, an interview or a setup
playbook. `spark org init --project <path>` only finishes an interrupted
installation and takes no profiles. Updates happen only when the operator asks.

## Context

Setting up an organization has two different kinds of step:

- **Mechanical:** create the org repo as a git repository; copy in the seed's
  core spec, the chosen profiles, the standards and the templates, and the
  bundled files ADR-0003 lists; and write the first README and changelog entry.
- **Judgment:** interview the operator, shape the spec, and add roles.

An org repo can exist before setup: Spark creates it on its first write, such as
a flow saved in the flow editor or a trigger created. factoryctl does the
mechanical part with a deterministic `init` command.

## Decision

- **A deterministic `spark org init --project <path> --profile <name>...`
  command** does the mechanical part. It creates the org repo if it does not
  exist, adds every file listed above that is missing, writes the README and the
  changelog's first entry if they are missing (ORG-030, ORG-035), commits, and
  reports what it created and what it skipped. It never overwrites a file, so it
  works the same on an org repo that already exists.
- **The Chief of Staff runs it**, because only the Chief of Staff writes the org
  repo (ORG-012). Before initialization that is Spark's bundled Chief of Staff.
- **The setup playbook** then runs as a mission, which the Chief of Staff runs,
  to do the judgment part. The operator starts setup by asking the Coordinator,
  which hands it to the Chief of Staff (ORG-013), and confirms the profiles
  before `spark org init` runs.

## Consequences

- Installation becomes reproducible and testable without a model.
- An org repo that already exists, such as Spark's own (ADR-0008), is filled in
  rather than skipped.
- Requires a Spark change: the command, plus the seed embedded in the build.

## Source

Proposed by the agent drafting the seed, 2026-10-28. Accepted by the operator,
2026-10-28: "I approve ADR-0003, ADR-0004, and ADR-0005 as written." Revised
before release so that it fills in an existing org repo, copies the standards,
templates and bundled files, and is run by the Chief of Staff; accepted by the
operator, 2026-10-29: "I also approve the ADR-0004 fix."
