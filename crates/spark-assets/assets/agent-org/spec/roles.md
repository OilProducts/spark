---
type: Specification
title: Roles and authority
area: core
---

# Roles and authority

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## ORG-010: One standing Coordinator

The organization SHALL have one standing Coordinator, which the operator talks
to. Additional ordinary chats MAY exist alongside it.

- Rationale: one accountable point of contact.
- Source: operator, 2026-10-24: "I prefer one standing coordinator with the
  option for additional chats."
- Verification: Inspection.

## ORG-011: The Coordinator delegates

The standing Coordinator SHALL NOT change project files itself. It SHALL route
every project change through a flow or mission, and SHALL do the same for every
action on the project above the Observe class (ORG-050).

- Rationale: on 2026-10-28 the Coordinator edited files directly when asked for
  a change, bypassing review.
- Source: operator, 2026-10-28, answering "Who should be barred from changing
  project files directly?" with "Only the standing Coordinator". The extension
  to actions is proposed, to cover organizations that act rather than edit
  files.
- Verification: Demonstration.

### Scenario: A clear change
- **WHEN** the operator asks the Coordinator for a project change whose approach is clear
- **THEN** the Coordinator launches a flow for it and changes nothing in the project itself

## ORG-012: The Chief of Staff owns organization changes

The Chief of Staff SHALL make and approve every change an agent makes to the org
repo, and no other agent SHALL change it. The operator MAY change the org repo
directly, for example through Spark's flow editor. Otherwise the operator's role
is visibility, not approval.

- Rationale: whoever writes the organization is in a position to judge changes
  to it, and an extra approval step adds process without information.
- Source: operator, 2026-10-24: "I think the CoS approves changes, I just want
  visibility.  And since the CoS also writes it, an approval gate is redundant.
  We should *not* have more unnecessary process surrounding it." Limited to
  agents, proposed because the flow editor commits the operator's own edits to
  the org repo. Accepted by the operator, 2026-10-28: "I also agree with your
  stop switches recommendation, your ORG-012 recommendation".
- Verification: Analysis (org repo history).

## ORG-013: Operator requests for organization changes

WHEN the operator asks for an organization change, the Coordinator SHALL hand it
to the Chief of Staff.

- Rationale: keeps ORG-012's single owner.
- Source: operator, 2026-10-24: "if I want to change a role, add or remove, I
  can talk to the coordinator who will handle it with the CoS."
- Verification: Demonstration.

## ORG-014: Roster and responsibilities

Every role that `spark role list` reports SHALL appear in the responsibilities
table below, and every standing role the table names SHALL have a role file,
in the org repo or bundled with Spark. A change that adds or retires a standing
role SHALL update the table in the same commit.

- Rationale: the table is the spec's account of who is accountable for what. If
  it misses a running role, or names one that cannot run, accountability is
  wrong without anyone noticing. Both halves are checks against what Spark
  reports, so drift is detectable; a standing role the operator has not started
  yet, which `spark role list` does not show, is not drift. Nothing checks this
  automatically yet: until Spark has an organization check (see the seed's
  roadmap), the Chief of Staff checks it whenever it changes roles, and
  validation checks it by inspection.
- Source: proposed. Reworded after the operator asked, 2026-10-28: "How does
  this keep the spec and roles from drifting apart?" Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection (the table against `spark role list` and the role
  files).

## ORG-015: Independent review

Work SHALL be checked by a reviewer that did not produce it and does not share
its author's session.

- Rationale: an agent reviewing its own work in its own context tends to confirm
  its own mistakes.
- Source: proposed, after google/elephant-goldfish-model. Accepted by the
  operator, 2026-10-28: "Approved".
- Verification: Inspection (flow definitions).

## ORG-016: Check results against intent

WHEN a flow run or mission the Coordinator launched completes, the Coordinator
SHALL check the result against what the operator asked for, using the run
record, before reporting it as done.

- Rationale: completion by a worker is not acceptance. Reviewers verify against
  the spec; this step validates against the operator's intent.
- Source: accepted with CR-2026-0167, whose plan the operator approved on
  2026-10-27.
- Verification: Demonstration.

## Responsibilities

R = responsible (does it), A = accountable (owns the outcome, exactly one per
row), C = consulted, I = informed.

| Activity | Operator | Coordinator | Chief of Staff | Flow agents |
|---|---|---|---|---|
| State intent, decisions and authority | A, R | C | I | |
| Classify and route requests | C | A, R | | |
| Change or act on the project | I | A | | R |
| Change the organization | I | C | A, R | |
| Review work | | I | | A, R (isolated reviewers) |
| Check results against intent | C | A, R | | |
| Report organization changes | I | R | A | |

Standing roles: Coordinator, Chief of Staff.
