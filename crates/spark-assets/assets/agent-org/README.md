# Agent organization seed

The starting point for an agent organization in Spark. An organization may manage
software, a dataset, an ongoing process, accounts or devices. The seed provides
a domain-neutral core spec, capability profiles, a setup playbook, vendored
standards, and templates for the organization's records. Each organization
copies the seed once and then evolves on its own.

## Project status

Draft, version 0.1.0. Spark does not use this directory yet. Nothing loads or
installs it. The operator has accepted every decision record, and every
requirement except ORG-001.

## Description

Spark already provides standing roles (a Coordinator the operator talks to, and a
Chief of Staff that owns the organization), flows, missions, triggers and a
per-project org repo. This seed defines what an organization built from those
parts must do, whatever it manages:

- keep organization, project and cross-boundary work apart;
- set authority by what an action can do: observe, change internally, or act
  externally;
- record the operator's decisions in their own words;
- make every organization change visible;
- check results against evidence and against what the operator asked for.

Capabilities only some organizations need come as profiles:

| Profile | For | Example organizations |
|---|---|---|
| `software` | Building and releasing software | Spark itself |
| `operations` | Ongoing processes: monitoring, analysis, reacting to events | Auction tracking, home devices |
| `external-effects` | Actions with financial, physical or other real-world consequences | Bidding, operating devices, deploying |

The seed adopts established standards rather than inventing conventions: BCP 14,
Semantic Versioning, Keep a Changelog and Make a README, plus ISO/IEC/IEEE 29148
requirement qualities, EARS, decision records and RACI. See
[standards/README.md](standards/README.md).

| Path | What it is |
|---|---|
| [spec/](spec/README.md) | Core spec: terms, areas, requirements `ORG-NNN`, responsibilities |
| [profiles/](profiles/) | `software` (`SW-`), `operations` (`OPS-`), `external-effects` (`EXT-`) |
| [playbook.md](playbook.md) | How to set an organization up from the seed |
| [standards/](standards/README.md) | Vendored standards and the requirements-writing guide |
| [decisions/](decisions/) | Decision records for the seed itself |
| [templates/](templates/) | Operator brief and decision record templates |
| [CHANGELOG.md](CHANGELOG.md) | Changes to the seed |

## Usage

Not wired in yet. The intended flow (ADR-0004):

1. `spark org init --project <path> --profile <name>...` copies the core and the
   chosen profiles into `<project>/.spark/agent-org/`.
2. A mission runs [playbook.md](playbook.md) to shape the organization with the
   operator.

Spark's own development will be the first organization (ADR-0008).

## Roadmap

- Implement the accepted decisions in Spark:
  - ADR-0003: copy the organization-owned files at installation;
  - ADR-0004: `spark org init`;
  - ADR-0005: move semantic rules out of `spark_control_surface` into the
    seed's roles.
  - ADR-0009: move triggers, playbooks and standing-role
    settings into the org repo.
- Wire the seed into Spark: embed it, add `spark org init`, and replace
  `playbooks/agent-org-setup.md` with this playbook.
- Add a check that every requirement ID cited in the playbook, profiles and roles
  exists.
- Install Spark's own organization from the seed (ADR-0008).

## Contributing

Changes follow the seed's own rules:

- requirements per [writing-requirements.md](standards/writing-requirements.md),
  in the area or profile file they belong to, with IDs never reused;
- decisions as records in [decisions/](decisions/);
- every change listed under `Unreleased` in [CHANGELOG.md](CHANGELOG.md);
- vendored standards replaced from their upstream source, never edited in place.

## Authors and acknowledgment

Drafted with the operator. It draws on:

- an agent-organization installation prompt shared by a friend of the operator
  (kept at `crates/spark-assets/assets/playbooks/agent-org-setup.md`);
- google/elephant-goldfish-model (independent review);
- tclasen/factoryctl (copy at initialization, operator brief, desired versus
  observed state);
- Fission-AI/OpenSpec (requirements with scenarios, single file ownership).

## License

The seed is part of Spark. The vendored standards keep their own licenses, listed
in [standards/README.md](standards/README.md).
