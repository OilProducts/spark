---
type: Specification
title: Actions and effect classes
area: core
---

# Actions and effect classes

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

Authority depends on what an action can do, not on who does it or which file it
touches. This applies to every organization: a software organization pushes and
deploys, an auction organization bids, a home organization switches devices.

## ORG-050: Effect classes

Every action the organization takes SHALL be classified by its effect:

| Class | What it covers | Examples |
|---|---|---|
| **Observe** | Reading only; changes nothing | Reading files, querying an API, checking a device's state |
| **Internal** | Changes inside the org repo or the project that version control or the organization itself can fully undo | Editing project files on a branch, updating working data, committing to the org repo |
| **External** | Anything that leaves the system or cannot be fully undone from inside it | Sending a message, spending money, placing a bid, actuating a device, deploying, pushing to a shared remote, deleting unversioned data |

- Rationale: a single rule for "changes" cannot tell a branch edit from a
  purchase, and the difference is what matters for authority.
- Source: operator, 2026-10-28, choosing to govern actions "by effect class in
  the core" (ADR-0007). The three classes and their boundaries are proposed.
  Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection.

## ORG-051: Authority per class

The operator brief SHALL record, for each effect class, and where useful for
particular kinds of action, what the organization may do without asking, what
needs the operator's authorization, and what it must never do. Where nothing is
recorded:

- Observe actions MAY proceed;
- Internal actions MAY proceed within the scope of authorized work;
- External actions SHALL NOT proceed without the operator's explicit
  authorization.

- Rationale: autonomy differs by organization and by kind of action, so it
  belongs to the operator, and the safe default is to ask before anything
  external.
- Source: proposed, implementing ADR-0007. Accepted by the operator, 2026-10-28:
  "Approved".
- Verification: Demonstration.

### Scenario: Unrecorded external action
- **WHEN** an agent is about to place a bid and the operator brief records no authority for bids
- **THEN** the agent does not place it, and the Coordinator asks the operator

## ORG-052: Uncertain class means the higher class

IF an action's effect class is uncertain, THEN the organization SHALL treat it as
the higher class.

- Rationale: a mistaken Observe that turns out External is the costly error.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Analysis.

## ORG-053: External actions are traceable

Every External action SHALL be traceable to the run or conversation that took it,
the authorization it relied on, and its result.

- Rationale: external effects are the ones that need an audit trail.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Analysis (run records).

## ORG-054: State the way back

Before an External action, the agent SHOULD state how it would be undone, or that
it cannot be.

- Rationale: a backout plan is standard change-management practice, and saying
  "this cannot be undone" makes the operator's authorization informed.
- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection (run records).
