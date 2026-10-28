---
type: Decision
title: What Spark keeps fixed
id: ADR-0005
status: Accepted
date: 2026-10-28
---

# ADR-0005: What Spark keeps fixed

## Status

Accepted

## Context

Spark's fixed instructions, which every agent receives and no organization can
edit, currently include semantic rules, such as "only the Chief of Staff changes
files in the org repo". Rules written in fixed prose are no more enforced than
the same rules in an editable role file. Their only difference is that an
organization cannot change them, which conflicts with organizations evolving
(ORG-040).

factoryctl draws the line at mechanics: its tool enforces structure, and every
workflow rule is project-owned text. Its test for scope: "if supporting its
workflow requires new backend domain logic, move that responsibility into
agent-interpreted Markdown."

## Decision

Spark's fixed layer holds only what the runtime enforces or needs in
order to run. That covers the control surface (CLI usage, paths), sessions,
isolation, visit limits, budgets and run records. Semantic rules move into the
seed's core spec and roles, where each organization owns them.

## Consequences

- One place for each rule, and an organization can evolve all of its rules.
- An organization can also weaken a rule. The protection is visibility: every
  spec change is recorded and reported (ORG-031, ORG-032).
- Requires moving text out of `spark_control_surface` into bundled role files.

## Source

Proposed by the agent drafting the seed, 2026-10-28. Accepted by the operator,
2026-10-28: "I approve ADR-0003, ADR-0004, and ADR-0005 as written."
