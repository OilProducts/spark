# Changelog

All notable changes to the agent organization seed are documented in this file.

The format is based on [Keep a Changelog](standards/keep-a-changelog-1.1.0.md),
and this seed adheres to [Semantic Versioning](standards/semver-2.0.0.md). The
seed's public interface is its core spec and profiles:

- **major:** removing a requirement or making one stricter;
- **minor:** adding a requirement or a profile;
- **patch:** clarifying wording.

## [Unreleased]

### Added

- Core spec (`spec/`, version 0.1.0), split by area:
  - boundaries (ORG-001 to ORG-006);
  - roles and authority, with a RACI table (ORG-010 to ORG-017);
  - records and evidence (ORG-020 to ORG-026);
  - organization changes (ORG-030 to ORG-035);
  - evolution (ORG-040, ORG-041);
  - actions and effect classes (ORG-050 to ORG-055).
- Capability profiles:
  - `software` (SW-001 to SW-006);
  - `operations` (OPS-001 to OPS-006);
  - `external-effects` (EXT-001, EXT-002, EXT-004 to EXT-006).
- Setup playbook (`playbook.md`): method only; the interview chooses profiles.
- Vendored standards:
  - RFC 2119 and RFC 8174, in full;
  - Semantic Versioning 2.0.0, in full;
  - Keep a Changelog 1.1.0, in part;
  - Make a README, in part.
- Requirements-writing guide (`standards/writing-requirements.md`): BCP 14,
  ISO/IEC/IEEE 29148 requirement qualities, EARS, decision records, RACI.
- Accepted decision records:
  - ADR-0001 vendor established standards;
  - ADR-0002 GitHub flow;
  - ADR-0003 single file ownership after installation;
  - ADR-0004 `spark org init`, then the setup mission;
  - ADR-0005 Spark keeps only mechanically enforced rules fixed;
  - ADR-0006 core plus capability profiles;
  - ADR-0007 authority by effect class;
  - ADR-0008 Spark is the first organization;
  - ADR-0009 the whole definition lives in the org repo;
  - ADR-0010 organization changes are reviewed, and only the operator grants
    authority.
- Templates for an organization's operator brief and decision records.
