---
type: Specification
title: Evolution
area: core
---

# Evolution

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
and only when, they appear in all capitals, as shown here.

## ORG-040: The organization evolves its own spec

The organization MAY change its spec, including adding standing roles, records
and flows, as the kind of work it does requires. Every such change SHALL satisfy
ORG-031 and ORG-032.

- Rationale: the seed is a starting point. Each organization is shaped by the
  work it manages.
- Source: operator, 2026-10-28: "he runs that prompt in a new project and each one
  evolves independently, and by "evolves" I mean things like the CoS may create a
  new persistent role for something, the orgs records and direction are molded by
  the *kind* of thing that its working on."
- Verification: Analysis (org repo history).

## ORG-041: Profiles can be added later

The organization MAY add or remove a profile after installation. Adding or
removing one SHALL satisfy ORG-031 and ORG-032.

- Rationale: an organization's work can grow a new kind of capability, such as
  a software project that starts operating a service.
- Source: operator, 2026-10-28, choosing "Core + capability profiles", described
  as "chosen at install and addable later" (ADR-0006).
- Verification: Inspection.
