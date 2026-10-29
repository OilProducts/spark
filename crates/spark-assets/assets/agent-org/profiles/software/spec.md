---
type: Specification
title: Software profile
profile: software
---

# Software profile

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

For organizations that build and release software. Adds to the
[core](../../spec/README.md); does not replace any of it.

## SW-001: Versioning released software

WHERE the project releases versioned software, the organization SHALL version it
with [Semantic Versioning 2.0.0](../../standards/semver-2.0.0.md), tag each
release on `main`, and keep a changelog following
[Keep a Changelog](../../standards/keep-a-changelog-1.1.0.md).

- Rationale: a SemVer number tells users whether a release can break them, and
  the changelog tells them what changed.
- Source: operator, 2026-10-28: "semantic versioning for software". Formerly
  ORG-033. Release tags on `main` from
  [ADR-0002](../../decisions/0002-git-branching-model.md), moved here from
  SW-006. An exception for projects that follow another convention was removed
  at the operator's direction, 2026-10-29: "Semver wins. Remove the
  contradiction."
- Verification: Inspection.

## SW-002: Project README

WHERE the project has no README, the organization SHOULD propose one using the
[Make a README](../../standards/make-a-readme.md) sections that apply, as project
work.

- Rationale: the README is the first thing a person or agent reads about a
  project.
- Source: operator, 2026-10-28 ("make a readme"). Formerly ORG-034.
- Verification: Inspection.

## SW-003: Project decisions

The project's own architecture decisions SHOULD be recorded as decision records
in the project repository, separately from the organization's decisions
(ORG-021).

- Rationale: design decisions about the software belong with the software, and
  outlive the change requests and conversations that produced them.
- Source: proposed by the agent drafting the seed, following ADR-0008. Accepted
  by the operator, 2026-10-28: "Approved".
- Verification: Inspection.

## SW-004: Changes go through review and the gate

Project changes SHALL be made through flows that review the work independently
(ORG-015) and pass the project's validation gate before they are merged. WHEN a
change's approach needs deciding, a design SHALL be drafted and reviewed before
it is implemented.

- Rationale: this is what Spark's `design-change`, `implement-change` and
  `merge-change` flows provide.
- Source: accepted with Spark's change requests CR-2026-0166 and CR-2026-0167
  (Spark commits db4925c4 and b42218c3), whose plans the operator approved on
  2026-10-27.
- Verification: Inspection (run records).

## SW-005: Recorded validation gate

The project's build and test gate SHALL be written down where agents read it (the
README or the project's agent instructions), as a command that can be run.

- Rationale: "tests pass" means nothing without an agreed gate.
- Source: proposed by the agent drafting the seed. Accepted by the operator,
  2026-10-28: "Approved".
- Verification: Inspection.

## SW-006: GitHub flow

Every change to the project SHALL be made on its own branch, reviewed there, and
merged into `main` only after passing the validation gate (SW-005). Changes SHALL
NOT be committed directly to `main`.

- Rationale: keeps `main` releasable, and every change reviewed.
- Source: [ADR-0002](../../decisions/0002-git-branching-model.md), accepted by
  the operator, 2026-10-28: "I approve ADR-0002 as written." Its rule on release
  tags moved to SW-001.
- Verification: Analysis (repository history).
