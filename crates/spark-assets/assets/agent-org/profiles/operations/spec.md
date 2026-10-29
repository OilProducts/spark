---
type: Specification
title: Operations profile
profile: operations
---

# Operations profile

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

For organizations that run an ongoing process: monitoring, analyzing data as it
arrives, or reacting to events, such as tracking auctions or looking after home
devices. Adds to the [core](../../spec/README.md).

## OPS-001: Triggers drive recurring work

Every recurring or event-driven activity SHALL be started by a Spark trigger
(schedule, poll, webhook or flow event) aimed at a role, mission or flow. An agent
SHALL NOT keep time by waiting or polling in its own conversation.

- Rationale: triggers persist across restarts and are visible; an agent's
  conversation is neither a clock nor a scheduler.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection.

## OPS-002: Recurring activities are described where they run

Each recurring activity SHALL be carried out by a flow, a standing role or a
mission that a trigger starts (OPS-001). It SHOULD be a flow when its steps are
the same every time, and a standing role or mission when each occurrence needs
judgment about what to do. Its description SHALL live where it runs:

- **Flow:** `flow.md` states the activity as a whole (its purpose, the trigger
  that starts it, and what it reports to whom). Each node states its own part:
  what it checks and what counts as abnormal, or what it does and that action's
  effect class (ORG-050). The routes carry what happens when a check finds
  something abnormal.
- **Standing role:** its role file states the same things for the activity.
- **Mission:** its playbook states them.

- Rationale: operations teams call this description a runbook. In Spark it
  belongs in the thing that carries the activity out, each part where it is
  carried out, so that it cannot drift from what actually runs. A flow suits
  routine procedure: its structure is fixed, its tool nodes can do deterministic
  checks without a model, and its graph can put a separate check before an
  External action (EXT-002). A role or mission suits work whose next step has to
  be decided each time.
- Source: proposed by the agent drafting the seed, revised after the operator
  asked, 2026-10-28: "What is a runbook? Do we already have that concept in
  spark?", "does all of that belong in the flow.md? or in the nodes that
  comprise the flow?" and "Why specifically a flow and not a mission from a
  playbook?" Approved by the operator, 2026-10-28: "Ok I approve that version."
- Verification: Inspection.

## OPS-003: Working data has a home and a lifetime

Each kind of working data (ORG-025) SHALL have a named location in the project
and a stated retention period.

- Rationale: data that accumulates without a stated lifetime grows without
  bound and goes stale unnoticed.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection.

## OPS-004: Repeat-safe activities

An activity that can run more than once for the same event (a retry, or a
duplicated trigger) SHALL NOT repeat its effects.

- Rationale: triggers and retries can deliver an event twice; a second bid or a
  second notification is a real cost.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Test.

### Scenario: Duplicate event
- **WHEN** the same auction-ending event triggers the activity twice
- **THEN** only one bid decision is made and at most one bid is placed

## OPS-005: Alerts

WHEN an activity detects a condition that the operator brief marks as an alert,
the organization SHALL notify the operator through the Coordinator within the
time the brief states.

- Rationale: an alert is worth sending only while the operator can still act on
  it.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Demonstration.

### Scenario: A price drops below the alert threshold
- **WHEN** the nightly price check finds a watched item below the price the operator brief marks as an alert
- **THEN** the operator hears about it from the Coordinator within the time the brief states

## OPS-006: A stopped activity is noticed

IF a recurring activity fails or stops running, THEN the organization SHALL
notice and tell the operator.

- Rationale: in an ongoing process, silence looks like "nothing happened".
- Source: proposed by the agent drafting the seed. The Chief of Staff's
  failed-run trigger covers failures; noticing an activity that stopped running
  entirely needs its own check. Accepted by the operator, 2026-10-28:
  "Approved".
- Verification: Test.

### Scenario: A check that silently stops
- **WHEN** the nightly price check's trigger is disabled by mistake and the check does not run
- **THEN** the organization notices the missed run and tells the operator
