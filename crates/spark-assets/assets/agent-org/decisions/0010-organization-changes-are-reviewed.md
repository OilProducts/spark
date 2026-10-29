---
type: Decision
title: Organization changes are reviewed
id: ADR-0010
status: Accepted
date: 2026-10-28
---

# ADR-0010: Organization changes are reviewed

## Status

Accepted

## Context

ORG-015 requires every piece of work to be checked by a reviewer that did not
produce it. Organization changes had no such check: ORG-012 had the Chief of
Staff write and approve them, and ADR-0002 had it change the org repo directly.
The Chief of Staff sees failures one at a time, and the easiest fix for each is
a new rule or step, so nothing stopped the organization from piling up process
for edge cases.

ORG-020 also kept anything an agent proposed "a proposal until the operator
confirms it". That made the operator the approver of every change, while ORG-012
said the operator's role is visibility, not approval.

## Decision

- The Chief of Staff prepares each organization change on a branch of the org
  repo, in a separate worktree, because Spark loads flows, roles and triggers
  from the org repo's checkout. It runs the bundled `organization/review-change`
  flow on the branch and merges only when the review approves (ORG-017). This
  replaces ADR-0002's statement that the Chief of Staff changes the org repo
  directly: GitHub flow now applies to the org repo too.
- The review is isolated and leans toward fewer, more general rules. It asks, in
  order, whether the change is needed, whether something existing covers it,
  whether it is general, whether it is the smallest change, and whether it
  conflicts with another rule. It never accepts removing authority, limits,
  safety constraints, credential handling or independent review as a
  simplification.
- Records (operator brief entries, decision records, the changelog) are reviewed
  with the change they record, not on their own. Role memory is not reviewed
  (ORG-026).
- If the Chief of Staff disagrees with a rejection, it takes the disagreement to
  the operator through the Coordinator.
- Who proposed a change and whether it is in force are separate questions.
  Records name the role that proposed each change and never attribute it to the
  operator (ORG-020). A change that passes review is in force, and the operator
  can overrule it at any time. Only the operator grants authority (ORG-055).

## Consequences

- Every organization change costs a review run.
- The operator is not a gate on ordinary changes; they settle disagreements and
  grant authority.
- Spark needs changes: the bundled Chief of Staff role has to work this way, and
  `spark trigger create`, `update` and `delete` commit straight to the org
  repo's checked-out branch, which skips the review.

## Source

Operator, 2026-10-28: "The organizational work needs a reviewer too, we should
probably have a workflow specifically for evaluating these. Without a strong
review biased towards generalization and simplicity I can easily see the CoS
ruining the org with neverending processes and rules for edge cases." The
mechanism, the separation of attribution from authority (ORG-020, ORG-055) and
the escalation of disagreements were proposed by the agent reviewing the seed.
Accepted by the operator, 2026-10-28: "I also agree with the entirety of what
you said for #1 and #2."
