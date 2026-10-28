---
type: Decision
title: The whole definition lives in the org repo
id: ADR-0009
status: Accepted
date: 2026-10-28
---

# ADR-0009: The whole definition lives in the org repo

## Status

Accepted

## Context

Only part of an organization's definition is in its org repo today: the
project's flows and roles, and standing roles' memory. The rest is in Spark's
own directories, shared by every project:

- triggers, in `$SPARK_HOME/config/triggers/<id>.toml`, each naming its project;
- playbooks, in `$SPARK_HOME/playbooks/`, seeded from Spark's packaged copies;
- which roles are standing, and their budgets, in Spark's record for each
  started role.

So a trigger the Chief of Staff adds changes when the agents work without
leaving anything in the org repo's history, and two projects cannot have
different playbooks.

## Decision

The org repo holds the organization's whole definition (ORG-025):

- triggers, as `triggers/<id>.toml`. A trigger belongs to the organization whose
  repo holds it, so it no longer names a project;
- playbooks, as `playbooks/<name>.md`, resolved project first, then bundled,
  like flows;
- which roles are standing, and their budgets.

Spark keeps runtime state outside the org repo: a trigger's last firing and next
run, a role's inbox and launch history, and whether a trigger or role is
currently stopped.

## Consequences

- The org repo's history covers every change to how the agents work.
- Projects can have different playbooks; Spark's bundled ones become a library.
- Stopping a trigger or role needs no organization change, so the operator can
  stop work at once (EXT-006).
- A trigger can no longer exist without a project.
- Spark needs changes to trigger storage, playbook resolution and role budgets.

## Source

The operator, 2026-10-28: "We should fix this so that triggers are part of the
org, just like flows, mission playbooks, standing roles, etc." The file layout
and keeping the stopped state with Spark are proposed by the agent drafting the
seed. Accepted by the operator, 2026-10-28: "ORG-025 and ADR-0009 are
approved. I also agree with your stop switches recommendation".
