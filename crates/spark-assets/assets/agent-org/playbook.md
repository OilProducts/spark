---
title: Agent organization setup
description: Set up this project's agent organization from the seed, shaped by an interview with the operator, then validate it against its spec.
status: Draft
---

# Agent organization setup

You are setting up this project's agent organization from the Spark seed. The
seed's [core spec](spec/README.md) and [profiles](profiles/) say what the
organization must be. This playbook is only the method. Follow the requirements
and cite their IDs.

## 1. Discover (read-only)

Find out what the project is before asking anything the environment already
answers. It may be a codebase, a dataset, an ongoing process, accounts or
devices. Look at:

- the project directory and its own instructions (`AGENTS.md`, `CLAUDE.md`);
- any README, changelog, build or test commands, and CI;
- existing data and scripts;
- `spark role list`, `spark flow list` and `spark trigger list`.

Change nothing.

## 2. Choose profiles

Propose the profiles that fit what you found, with your reasons, and let the
operator confirm:

- `software` for building and releasing software;
- `operations` for an ongoing process;
- `external-effects` for actions with financial, physical or other real-world
  consequences.

Record their answer in the operator brief.

## 3. Install the seed

If the org repo `<project>/.spark/agent-org/` does not exist, the Chief of Staff
creates it as a git repository and copies in the core spec, the chosen profiles,
the standards and the templates. It records the seed version and profiles in a
new `CHANGELOG.md` (ORG-035) and writes a `README.md` (ORG-030). Every write to
the org repo goes through the Chief of Staff (ORG-012).

## 4. Interview the operator

Ask one topic per turn and end your turn after each question. Record the
operator's answers verbatim and dated in `operator-brief.md` (ORG-020). Don't ask
anything discovery or the brief already answers.

Cover, in this order:

1. **Purpose**: what the project is for and what success looks like.
2. **Boundaries**: what belongs to the project, what is off limits, and where
   working data lives (ORG-025).
3. **Authority**: for each effect class, what the organization may do unasked,
   what needs the operator, and what it must never do (ORG-051). With
   `external-effects`, also limits and safety constraints (EXT-001, EXT-004).
4. **Roles**: which standing roles beyond the Coordinator and Chief of Staff the
   work needs.
5. **Profile specifics**:
   - `software`: versioning, changelog, README, validation gate, GitHub flow
     (SW-001 to SW-006);
   - `operations`: activities, schedules, alerts and data retention (OPS-001 to
     OPS-006).
6. **Reporting**: how and when the operator wants to hear about work and about
   organization changes.

## 5. Shape the organization

From the answers, the Chief of Staff adapts the spec, adds standing roles with
their rows in the responsibilities table (ORG-014), adds the triggers, and the
flows, roles or missions, for recurring work (OPS-001, OPS-002), and writes
decision records (ORG-021).
Each change is committed with its reason and listed in the changelog (ORG-031).
Spec changes are reported to the operator (ORG-032).

## 6. Validate

Carry out every requirement whose verification is Demonstration or Test as a real
request to the standing Coordinator. Judge what actually happened from the run
records and the org repo history (ORG-023). Record each result, pass or fail,
with its evidence.

Never validate an External action by taking it. Use a preview where one exists
(EXT-005), or ask the operator.

## 7. Accept

Report to the operator: what was set up, what passed, what failed, and what Spark
cannot do yet. The setup is accepted only when the operator says so. Close the
mission with the result.
