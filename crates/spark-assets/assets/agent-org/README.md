# Agent organization seed

The starting point for an agent organization in Spark. An organization may manage
software, a dataset, an ongoing process, accounts or devices. The seed provides
a domain-neutral core spec, capability profiles, vendored standards, and
templates for the organization's records. Each organization receives a copy of
the seed once and then evolves on its own.

## Project status

Draft, version 0.1.0. Spark embeds the seed and installs it automatically
when a project is registered. ADR-0009 is in place: playbooks, standing-role
settings and triggers resolve from the project's `.spark/definitions/`. The
operator has accepted every decision record, and every requirement except
ORG-001.

## Description

Spark already provides standing roles (a Coordinator the operator talks to, and a
Chief of Staff that owns the organization), flows, missions, triggers and a
per-project definitions directory, `<project>/.spark/definitions/`, with
working data beside it in `<project>/.spark/state/`. This seed defines what an organization built from those
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
| [standards/](standards/README.md) | Vendored standards and the requirements-writing guide |
| [decisions/](decisions/) | Decision records for the seed itself |
| [templates/](templates/) | Operator brief, decision record, README and changelog templates |
| [CHANGELOG.md](CHANGELOG.md) | Changes to the seed |

## Usage

When Spark registers a project, it installs, once, into
`<project>/.spark/definitions/` whatever is missing of:

- `spec/`, `profiles/` (every profile), `standards/`, `templates/` and
  `decisions/`;
- every bundled flow, role and mission playbook;
- a README and changelog filled from [templates/](templates/) with the Spark and
  seed versions and the profiles (ORG-030, ORG-035).

It never overwrites an existing file, asks no questions, runs no Git and
creates no triggers. It writes `.initialized` last; once that exists Spark
installs nothing more there, even after an upgrade. `spark org init --project
<path>` runs the same installation locally to finish an interrupted one. The
seed's own README and changelog are not installed.

Later changes are made only when the operator asks, including taking files from
a newer Spark. The Chief of Staff prepares each organization change as a
unified diff under `<project>/.spark/state/reviews/` and applies it only after
`organization/review-change` approves it (ADR-0010).

Spark's own development will be the first organization (ADR-0008).

## Roadmap

- Finish the accepted decisions in Spark:
  - ADR-0003: Spark still falls back to bundled flows and roles the
    definitions do not have; stop using bundled files the organization has not
    adopted;
  - ADR-0005: move semantic rules out of `spark_control_surface` into the
    bundled standing roles;
  - ADR-0010: the Chief of Staff's instructions route its changes through
    `organization/review-change`, but `spark trigger create`, `update` and
    `delete` still write straight to the definitions, which skips the review.
- Add a check that every requirement ID cited in the profiles and roles
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
  (preserved in Spark's Git history);
- google/elephant-goldfish-model (independent review);
- tclasen/factoryctl (copy at initialization, operator brief, desired versus
  observed state);
- Fission-AI/OpenSpec (requirements with scenarios, single file ownership).

## License

The seed is part of Spark. The vendored standards keep their own licenses, listed
in [standards/README.md](standards/README.md).
