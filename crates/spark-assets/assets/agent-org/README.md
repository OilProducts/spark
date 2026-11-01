# Agent organization seed

The starting point for an agent organization in Spark. An organization may manage
software, a dataset, an ongoing process, accounts or devices. The seed provides
a domain-neutral core spec, capability profiles, a setup playbook, vendored
standards, and templates for the organization's records. Each organization
copies the seed once and then evolves on its own.

## Project status

Draft, version 0.1.0. Spark embeds the seed and installs it with
`spark org init`; the bundled `agent-org-setup` playbook is this directory's
[playbook.md](playbook.md). ADR-0009 is in place: playbooks, standing-role
settings and triggers (Spark commit 99c9d051) resolve from the org repo. The
operator has accepted every decision record, and every requirement except
ORG-001.

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
| [templates/](templates/) | Operator brief, decision record, README and changelog templates |
| [CHANGELOG.md](CHANGELOG.md) | Changes to the seed |

## Usage

The flow (ADR-0004):

1. The operator asks the Coordinator to set up the organization, and the
   Coordinator hands it to the Chief of Staff (ORG-013).
2. The Chief of Staff runs [playbook.md](playbook.md) as a mission. Once the
   operator confirms the profiles, it runs
   `spark org init --project <path> [--profile <name>]...`, and then shapes the
   organization with the operator. Each later organization change is prepared on
   a branch in a separate worktree and merged only after
   `organization/review-change` approves it (ADR-0010).

`spark org init` runs locally and needs no server. Into
`<project>/.spark/agent-org/` it adds whatever is missing of:

- `spec/`, `standards/`, `templates/` and each chosen profile's directory;
- the bundled flow folders named in the `flows` front matter of
  [spec/README.md](spec/README.md) and of each chosen profile's `spec.md`;
- the bundled standing roles and every role those flows name;
- a README and changelog filled from [templates/](templates/) with the Spark and
  seed versions and the profiles (ORG-030, ORG-035).

It never overwrites an existing file, so an existing README or changelog is kept
as is. It creates the org repo if needed, commits only the files it created, and
prints each file it created or skipped. The seed's own README, changelog,
playbook and decisions are not installed.

Spark's own development will be the first organization (ADR-0008).

## Roadmap

- Finish the accepted decisions in Spark:
  - ADR-0003: Spark still falls back to bundled flows and roles the org repo
    does not have; stop using bundled files the organization has not adopted;
  - ADR-0005: move semantic rules out of `spark_control_surface` into the
    bundled standing roles;
  - ADR-0010: the Chief of Staff's instructions route its changes through
    `organization/review-change`, but `spark trigger create`, `update` and
    `delete` still commit straight to the org repo's checked-out branch, which
    skips the review.
- Add a check that every requirement ID cited in the playbook, profiles and roles
  exists, and validate an installed organization against its spec.
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
  (preserved in Git history at
  `crates/spark-assets/assets/playbooks/agent-org-setup.md`; the bundled
  playbook now comes from this seed);
- google/elephant-goldfish-model (independent review);
- tclasen/factoryctl (copy at initialization, operator brief, desired versus
  observed state);
- Fission-AI/OpenSpec (requirements with scenarios, single file ownership).

## License

The seed is part of Spark. The vendored standards keep their own licenses, listed
in [standards/README.md](standards/README.md).
