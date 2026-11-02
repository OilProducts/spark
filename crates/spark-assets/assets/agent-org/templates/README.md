# Agent organization

This directory is the project's agent organization: the requirements, roles,
flows, playbooks, triggers and records Spark's standing roles work from. The
project owns every file here. Spark runs no Git in it; whether to version it is
the operator's choice. Working data, such as role memory and run checkouts,
lives beside it in `.spark/state/`.

## Description

The organization follows the [core specification](spec/README.md). Every
capability profile was copied as reference material:
{{profiles}}. Its requirements are written according to the vendored
[standards](standards/).

The operator's purpose for the organization and the authority it holds are
recorded in the operator brief when the operator states them.

## Installation

Installed automatically when the project was registered, from
Spark {{spark_version}} and seed {{seed_version}}. Installation copies only
missing files and happens once; `.initialized` marks it done. Spark never updates these
files on its own. To take changes from a newer Spark, ask for the additions,
replacements or merges you want.

## Usage

Ask the Coordinator to change the organization. The Coordinator hands
organization work to the Chief of Staff, which records the operator's decisions.

## Contributing

Organization changes are prepared as a diff under `.spark/state/reviews/` and
take effect only after an independent review approves them. Every change is
recorded in [CHANGELOG.md](CHANGELOG.md).

## Project status

Installed.
