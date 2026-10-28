---
type: Specification
title: Boundaries
area: core
---

# Boundaries

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## ORG-001: Classify work by its target

The organization SHALL classify every substantive request as organization,
project or cross-boundary work, by the intended target rather than by which
agent does it. The test is what the request would change if it were done:

- only how the agents work (their roles, flows, triggers, spec and records):
  **organization** work;
- only what they manage (code, data, documents, accounts, devices): **project**
  work;
- both: **cross-boundary** work.

| | Organization work | Project work | Cross-boundary work |
|---|---|---|---|
| Software | "Have a second reviewer check every design before it is implemented" (a flow) | "Fix the login timeout bug" (code) | "Make the agents run the security scanner, and add it to CI" (a flow, and the CI configuration) |
| Auctions | "Use a cheaper model for the nightly price check" (a node's settings) | "Add vintage cameras to the watchlist" (working data) | "Start watching a second auction site" (a new flow and trigger, and an account and data for the site) |
| Home devices | "Report to me once a day instead of after every change" (a role) | "Set the thermostat's night temperature to 65°F" (a device setting) | "Alert me when the furnace stops responding" (a new monitoring flow, and the furnace's integration) |

- Rationale: if the two are confused, maintaining the organization can silently
  alter the project, and project work can silently rewrite how the agents
  operate.
- Source: proposed, adapted from the seed installation prompt (Parts 1-2).
  Examples revised after the operator, 2026-10-28: "I want more clear examples.
  The Scenarios do not immediately make sense to me." Awaiting the operator's
  review of this wording.
- Verification: Demonstration.

### Scenario: A change to how the agents work
- **WHEN** the operator asks the Coordinator to use a cheaper model for the nightly price check
- **THEN** the request is classified as organization work, because only the settings of a node in the price-check flow change

### Scenario: A change to what is managed
- **WHEN** the operator asks to add vintage cameras to the watchlist
- **THEN** the request is classified as project work, because only the watchlist, which is working data in the project, changes

### Scenario: A change to both
- **WHEN** the operator asks to start watching a second auction site
- **THEN** the request is classified as cross-boundary work, because it needs a new flow and trigger (how the agents work) and an account and data for the site (what they manage)

## ORG-002: Ask when the target is ambiguous

WHEN a request's target is ambiguous, the Coordinator SHALL ask the operator
which is meant before any work starts.

- Rationale: the broadest reading is rarely the intended one.
- Source: proposed, adapted from the seed installation prompt (2.2). Accepted by
  the operator, 2026-10-28: "Approved".
- Verification: Demonstration.

### Scenario: Ambiguous request
- **WHEN** the operator asks to "improve our testing"
- **THEN** the Coordinator asks whether that means the project's tests, the agents' testing procedures, or both, and does nothing until answered

## ORG-003: Authority does not cross the boundary

Authorization to change one of the organization and the project SHALL NOT be
treated as authorization to change the other.

- Rationale: discovery is not authorization, and a recommendation is not
  permission.
- Source: proposed, adapted from the seed installation prompt (2.3, 2.4, 22).
  Accepted by the operator, 2026-10-28: "Approved".
- Verification: Test.

### Scenario: Project work finds an organization weakness
- **WHEN** an agent working on the project finds a weakness in its own role instructions
- **THEN** it records a proposal for the Chief of Staff and does not edit the role file

## ORG-004: Stop at the boundary

IF work on one side requires a change to the other, THEN the agent SHALL stop
that change, record what is needed and why, and raise it with the Coordinator
instead of making it.

- Rationale: a silent cross-boundary change is what ORG-003 exists to prevent.
- Source: proposed, adapted from the seed installation prompt (2.6). Accepted by
  the operator, 2026-10-28: "Approved".
- Verification: Demonstration.

## ORG-005: Split cross-boundary work

A cross-boundary request SHALL be split into an organization part and a project
part, each with its own acceptance. The whole SHALL be accepted only when both
parts are.

- Rationale: success on the project side does not show that the organization
  change works, and the reverse.
- Source: proposed, adapted from the seed installation prompt (2.5, 2.7).
  Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection.

## ORG-006: State the target

Every mission objective and flow-run objective SHALL state whether its target is
the organization, the project, or both.

- Rationale: Spark has no field for the target, so the objective carries it.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection.
