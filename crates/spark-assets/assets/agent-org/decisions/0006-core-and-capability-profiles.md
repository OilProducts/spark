---
type: Decision
title: Core plus capability profiles
id: ADR-0006
status: Accepted
date: 2026-10-28
---

# ADR-0006: Core plus capability profiles

## Status

Accepted

## Context

Not every organization builds software. The operator's examples include an
organization that tracks auctions for resale and one that manages home devices
on the network. Other organizations run ongoing processes, such as analyzing
data or reacting to events.

The first draft built software conventions (versioning, README, branching) into
the core.

## Decision

The seed has a domain-neutral core that every organization gets, plus
capability profiles that can be combined: `software`, `operations` and
`external-effects`. An organization chooses its profiles at installation and can
add or remove them later (ORG-041). Profiles describe capabilities, not
industries, so an auction organization is `operations` plus `external-effects`.

## Consequences

- The core stays small and applies everywhere.
- Each profile keeps its own ID prefix (`SW-`, `OPS-`, `EXT-`).
- Requirements that turn out to be general move from a profile into the core.

## Source

Operator, 2026-10-28. Asked how the seed should accommodate non-software
organizations, chose "Core + capability profiles", described as "Domain-neutral
core every org gets, plus composable capability profiles (operations,
external-effects, software) chosen at install and addable later." Context from
the operator, 2026-10-28: "not all agent-orgs will be building software. Some
may be maintaining an ongoing process, such as analyzing some set of data, or
reacting to something. I may have an agent org track auctions for resale, or
manage my network connected home devices for example."
