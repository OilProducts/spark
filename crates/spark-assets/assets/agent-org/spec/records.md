---
type: Specification
title: Records and evidence
area: core
---

# Records and evidence

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## ORG-020: Operator brief and attribution

The organization SHALL keep an operator brief that records the operator's
decisions in their own words, with dates. Every requirement and decision SHALL
cite its source, and anything an agent proposed SHALL be labelled as a proposal
until the operator confirms it.

- Rationale: agent proposals have twice been recorded as operator decisions.
- Source: proposed, after factoryctl's operator brief. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection.

## ORG-021: Decision records

The organization's decisions SHALL be recorded as numbered decision records in
the format of [writing-requirements.md](../standards/writing-requirements.md).

- Source: proposed. Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection.

## ORG-022: Requirement format

Requirements the organization writes SHALL follow
[writing-requirements.md](../standards/writing-requirements.md), including BCP 14
keywords where a requirement level is meant.

- Source: operator, 2026-10-28: "I'd like to stick to well tested standards when
  reasonable, that means 2119 compliant where appropriate".
- Verification: Inspection.

## ORG-023: Claims need evidence

An agent SHALL NOT report a capability as working, a check as passed, an action
as taken, or a run as done unless it can point to the evidence: the run record,
the command and its output, a file and line, or the external system's response.

- Rationale: a documented capability is not an operational one.
- Source: proposed, adapted from the seed installation prompt (4.1, 18).
  Accepted by the operator, 2026-10-28: "Approved".
- Verification: Analysis.

## ORG-024: Credentials

Credentials SHALL NOT be stored in any git repository, neither the project's nor
the org repo. They MAY be kept near the project in an untracked location readable
only by the operator's account, such as a `credentials/` directory under the
project's `.spark/` (which the project repository ignores and the org repo does
not contain) with file mode 0600, or in the operating system's credential store.
Organization files SHALL refer to a credential by name. Agents SHALL pass a
credential to the tool that needs it by reference, as an environment variable or
a file path, and SHALL NOT read its value into their conversation.

- Rationale: git history keeps a secret forever, even after the file is deleted.
  Spark records conversations and tool output, so a credential an agent reads
  ends up in those records. Spark already keeps its own provider keys this way,
  in `$SPARK_HOME/config/provider.env` with mode 0600, outside any repository.
  Almost every organization holds some credential, such as a CI token, an API
  key or a device hub login, so the rule belongs in the core.
- Source: proposed, adapted from the seed installation prompt (17). Replaced by
  the text of EXT-003, which the operator approved on 2026-10-28, after the
  operator asked, 2026-10-28: "Does this still match with what we decided about
  them being in version control?" "Records and flows" is widened to
  "Organization files". Accepted by the operator, 2026-10-28: "Approved".
- Verification: Inspection (tracked files and file modes) and Analysis
  (conversation records).

## ORG-025: The org repo holds the whole definition

The org repo SHALL hold the organization's whole definition: its spec and
records, its roles and which of them are standing, its flows, its playbooks and
its triggers. Flows, roles and playbooks the organization has not copied MAY
instead resolve from Spark's bundled library (ADR-0003).

The org repo SHALL NOT hold working data, such as observations, datasets,
inventories and histories; that SHALL live in the project. Runtime state, such
as a trigger's last firing, a role's inbox, and whether a trigger or role is
currently stopped, SHALL stay with Spark.

- Rationale: the org repo's history is the record of how the agents work
  (ORG-031). Anything that changes how they work but lives elsewhere, such as a
  trigger in Spark's configuration, changes the organization without a record.
  Working data in the repo would bury organization changes in data churn.
  Stopping stays outside so that the operator can stop work at once without an
  organization change (EXT-006).
- Source: proposed. Triggers, playbooks and standing roles added at the
  operator's direction, 2026-10-28: "We should fix this so that triggers are
  part of the org, just like flows, mission playbooks, standing roles, etc."
  (ADR-0009). Keeping the stopped state with Spark was proposed. Accepted by the
  operator, 2026-10-28: "ORG-025 and ADR-0009 are approved."
- Verification: Inspection.
