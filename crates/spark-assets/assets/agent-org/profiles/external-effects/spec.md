---
type: Specification
title: External effects profile
profile: external-effects
---

# External effects profile

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

For organizations whose External actions (ORG-050) have financial, physical or
other real-world consequences: spending money, bidding, sending messages,
operating devices, deploying. Adds to the [core](../../spec/README.md).

## EXT-001: Limits

The operator brief SHALL state limits for each kind of External action the
organization takes. The limits fit the kind of action. Examples include amounts
per action and per period for spending, permitted devices and operations for
devices, and permitted recipients for messages; these examples are not a
complete list.

- Source: proposed. Amended at the operator's request, 2026-10-28: "I dont want
  to limit the limitations to only those categories, but yes." Approved as
  written by the operator, 2026-10-28: "EXT-001 and EXT-003 are now approved as
  written.  As is EXT-005".
- Verification: Inspection.

## EXT-002: Checked before taken

An External action SHALL be checked against its limits before it is taken, by a
check or reviewer separate from the agent proposing the action.

- Rationale: the maker/checker separation (ORG-015) applied to actions rather
  than to work products.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Test.

### Scenario: Over the limit
- **WHEN** an agent proposes a bid above the per-auction limit in the operator brief
- **THEN** the check refuses it, the bid is not placed, and the operator is told

## EXT-004: Safety constraints

WHERE actions affect physical devices or anyone's safety, the operator brief SHALL
list constraints that no action may violate, and the organization SHALL NOT take
an action that would violate one, whatever its authorization.

- Rationale: some limits are not trade-offs, for example a minimum temperature,
  or never unlocking a door remotely.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Test.

## EXT-005: Preview first

WHERE an external system offers a preview, simulation or dry run, the organization
SHOULD use it before the first action of a new kind.

- Rationale: a preview catches wrong parameters, the wrong account or the wrong
  device before the effect is real. Examples: `terraform plan`, a marketplace or
  payment API's sandbox, an order-validation call, a message sent to oneself
  first. It is SHOULD because many systems have no preview, and it applies to the
  first action of each kind so that routine actions are not slowed.
- Source: proposed. Approved by the operator, 2026-10-28: "EXT-001 and EXT-003
  are now approved as written.  As is EXT-005".
- Verification: Inspection (run records).

## EXT-006: One way to stop everything

The operator SHALL be able to stop all External actions at once, and the org
repo's README SHALL say how.

- Rationale: when something goes wrong in the world, stopping comes before
  diagnosing.
- Source: proposed. In Spark today this means stopping the responsible standing
  roles and disabling their triggers. Accepted by the operator, 2026-10-28:
  "Approved".
- Verification: Demonstration.

EXT-003 (credentials) moved to the core as ORG-024 before the seed was first
released. It is not reused.
