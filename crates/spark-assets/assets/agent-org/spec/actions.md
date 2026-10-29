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
| **Internal** | Changes that stay inside the org repo or the project, whether or not they can be undone | Editing or deleting files, updating or deleting working data, clearing caches and build output, writing logs and records, committing to the org repo |
| **External** | Anything that reaches people, money, devices or systems outside the org repo and the project | Sending a message, spending money, placing a bid, actuating a device, deploying, pushing to a shared remote |

- Rationale: a single rule for "changes" cannot tell a branch edit from a
  purchase, and the difference is what matters for authority. The line is
  reach, not reversibility: a change that stays inside affects no one outside,
  and requiring every internal write to be undoable would mean journaling every
  tool call. Deleting is Internal so that cleanup needs no one's permission and
  cruft does not accumulate; an organization holding something it must never
  lose says so in its operator brief (ORG-051).
- Source: operator, 2026-10-28, choosing to govern actions "by effect class in
  the core" (ADR-0007). The three classes and their boundaries were proposed by
  the agent drafting the seed. Accepted by the operator, 2026-10-28: "Approved".
  Redrawn by reach instead of reversibility at the operator's direction,
  2026-10-29: "I dont think every tool call you make needs to be recorded in an
  atomic journal" and, on deletion, "I dont want to be involved. I think this
  behaviour was trained into the frontier models as a response to a couple high
  profile tweets about an LLM deleting way too many things and now its far over
  corrected."
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
- Source: proposed by the agent drafting the seed, implementing ADR-0007.
  Accepted by the operator, 2026-10-28: "Approved".
- Verification: Demonstration.

### Scenario: Unrecorded external action
- **WHEN** an agent is about to place a bid and the operator brief records no authority for bids
- **THEN** the agent does not place it, and the Coordinator asks the operator

## ORG-052: Uncertain class means the higher class

IF an action's effect class is uncertain, THEN the organization SHALL treat it as
the higher class.

- Rationale: a mistaken Observe that turns out External is the costly error.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Analysis.

## ORG-053: External actions are traceable

Every External action SHALL be traceable to the run or conversation that took it,
the authorization it relied on, and its result.

- Rationale: external effects are the ones that need an audit trail.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Analysis (run records).

## ORG-054: State the way back

Before an External action, the agent SHOULD state how it would be undone, or that
it cannot be.

- Rationale: a backout plan is standard change-management practice, and saying
  "this cannot be undone" makes the operator's authorization informed.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection (run records).

## ORG-055: Only the operator grants authority

A change to the organization SHALL NOT widen what it may do without asking,
loosen a limit or safety constraint, or weaken a requirement that protects them,
unless the operator brief records the operator's approval of that change.

- Rationale: authority already comes only from the operator brief (ORG-051),
  which holds only the operator's words. Without this rule the organization
  could widen its authority another way, for example by deleting a safety
  requirement, and ORG-017's review, which favors removing process, could accept
  that as a simplification.
- Source: proposed by the agent reviewing the seed, after a consistency review
  found that ORG-020 made the operator approve every agent proposal while
  ORG-012 made the operator's role visibility. Accepted by the operator,
  2026-10-28: "I also agree with the entirety of what you said for #1 and #2."
  (ADR-0010).
- Verification: Inspection (org repo history against the operator brief).
