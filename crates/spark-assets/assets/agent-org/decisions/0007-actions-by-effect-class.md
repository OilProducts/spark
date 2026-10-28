---
type: Decision
title: Authority by effect class, in the core
id: ADR-0007
status: Accepted
date: 2026-10-28
---

# ADR-0007: Authority by effect class, in the core

## Status

Accepted

## Context

The first draft governed project *changes* (files). Organizations that run
processes also *act*: they bid, buy, message and operate devices. Software
organizations act too, when they push or deploy. A rule about files cannot tell
a branch edit from a purchase.

## Decision

The core classifies every action by its effect class, Observe, Internal or
External, and authority is set per class in the operator brief (ORG-050 to
ORG-054). The `external-effects` profile adds specifics such as spending limits,
checks before an action is taken, and safety constraints.

## Consequences

- One authority model for every organization.
- External actions default to requiring the operator's authorization until the
  operator brief says otherwise.

## Source

Operator, 2026-10-28. Asked "Should actions with external effects (spending,
physical devices, deploys, irreversible operations) be governed by effect class
in the core?", chose "Yes, in core". The three classes and their boundaries were
proposed by the agent drafting the seed.
