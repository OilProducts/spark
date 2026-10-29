---
type: Specification
title: Organization changes
area: core
---

# Organization changes

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## ORG-030: README and changelog

The org repo SHALL contain a `README.md` using the
[Make a README](../standards/make-a-readme.md) sections that apply, and a
`CHANGELOG.md` following [Keep a Changelog](../standards/keep-a-changelog-1.1.0.md).

- Rationale: the README tells an agent or person what the organization is and
  how to work with it, and a changelog in a known format is where the operator
  sees what changed (ORG-031).
- Source: operator, 2026-10-28: "the "Keep a changelog" and "make a readme"
  industry standards".
- Verification: Inspection.

## ORG-031: Every organization change is visible

Every change to the org repo SHALL be committed with its reason, and listed under
`Unreleased` in the organization's `CHANGELOG.md`.

- Rationale: this is how the operator gets the visibility ORG-012 promises.
- Source: proposed by the agent drafting the seed, serving the operator's
  2026-10-24 requirement for visibility. Accepted by the operator, 2026-10-28:
  "Approved".
- Verification: Inspection.

## ORG-032: Spec changes are reported

WHEN the organization changes its spec or its profiles, the Chief of Staff SHALL
tell the operator, through the Coordinator, what changed and why.

- Rationale: changes to the spec are where drift would hide.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Demonstration.

### Scenario: A requirement changes
- **WHEN** the Chief of Staff merges a change to one of the organization's requirements
- **THEN** the operator hears from the Coordinator what changed and why, with the commit

## ORG-035: Seed version

The organization SHALL record, in its changelog's first entry, the seed version
and the profiles it was installed with.

- Rationale: lets the Chief of Staff review what changed in the seed since then.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection.

ORG-033 and ORG-034 moved to the software profile as SW-001 and SW-002 before
the seed was first released. They are not reused.
