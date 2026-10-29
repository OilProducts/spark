---
type: Specification
title: Agent organization core specification
version: 0.1.0
status: Draft
---

# Agent organization core specification

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## Purpose

This specification states what must hold for every agent organization in Spark,
whatever it manages: software, a dataset, an ongoing process, accounts or
devices. Capabilities that only some organizations need live in
[profiles](../profiles/). Each organization copies the core and the profiles it
chose at installation, owns its copy from then on, and evolves it as its work
requires (ORG-040).

The spec says *what* must hold. *How* the organization runs lives in the role
and flow files that carry each requirement out. The installation method lives in
the [playbook](../playbook.md). Requirements follow
[writing-requirements.md](../standards/writing-requirements.md).

## Terms

- **Operator**: the human the organization works for.
- **Organization**: the organization's own definition and records (spec, roles,
  flows, playbooks, triggers, decisions, operator brief), kept in the project's
  org repo, `<project>/.spark/agent-org/` (ORG-025), with its standing roles'
  memory (ORG-026).
- **Project**: whatever the organization manages outside its org repo. That may
  be a codebase, a dataset, an ongoing process, accounts or devices, together
  with the actions taken on them. This is the same "project" Spark registers: a
  directory, which may hold code, data, configuration or scripts.
- **Organization work / project work / cross-boundary work**: work whose
  intended target is the organization, the project, or both.
- **Action**: anything the organization does to the project or through it. An
  action has an effect class (ORG-050).
- **Standing role**: a role with its own long-lived conversation and memory: the
  Coordinator, the Chief of Staff, and any role the organization adds.
- **Seed**: the starting core, profiles, playbook, standards and templates.

## Areas

| File | Requirements | Covers |
|---|---|---|
| [boundaries.md](boundaries.md) | ORG-001 to ORG-006 | Organization, project and cross-boundary work |
| [roles.md](roles.md) | ORG-010 to ORG-017 | Standing roles, authority, review of work and of organization changes, responsibilities (RACI) |
| [records.md](records.md) | ORG-020 to ORG-026 | Operator brief, decisions, requirement format, evidence, credentials, working data, role memory |
| [changes.md](changes.md) | ORG-030 to ORG-035 | README, changelog, visibility of organization changes, seed version |
| [evolution.md](evolution.md) | ORG-040 to ORG-041 | How the organization changes its own spec and profiles |
| [actions.md](actions.md) | ORG-050 to ORG-055 | Effect classes, the authority each needs, and who grants it |

## Profiles

| Profile | Prefix | For organizations that |
|---|---|---|
| [software](../profiles/software/spec.md) | `SW-` | build and release software |
| [operations](../profiles/operations/spec.md) | `OPS-` | run an ongoing process: monitoring, analysis, reacting to events |
| [external-effects](../profiles/external-effects/spec.md) | `EXT-` | take actions with financial, physical or other external consequences |

## Identifiers

Each requirement has a stable ID that is never reused or renumbered, whichever
file it lives in. Core IDs use the `ORG-` prefix, in a numeric range per area.
Profile IDs use the profile's prefix. A withdrawn requirement keeps its ID and is
marked withdrawn.
