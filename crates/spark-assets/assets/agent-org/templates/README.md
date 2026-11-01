# Agent organization

This directory is the project's agent organization: the requirements, roles,
flows, triggers and records Spark's standing roles work from. It is its own Git
repository, and its history is the organization's audit trail.

## Description

The organization follows the [core specification](spec/README.md) and the
capability profiles it adopted: {{profiles}}. Its requirements are written
according to the vendored [standards](standards/).

The operator's purpose for the organization and the authority it holds are still
to be established during setup.

## Installation

Initialized by `spark org init` from Spark {{spark_version}} and
seed {{seed_version}}. The initializer adds only missing files; from then on
the organization owns every file here.

## Usage

Ask the Coordinator to set up or change the organization. The Coordinator hands
organization work to the Chief of Staff, which runs the setup playbook and
records the operator's decisions.

## Contributing

Organization changes are prepared on a branch and take effect only after an
independent review approves them. Every change is recorded in
[CHANGELOG.md](CHANGELOG.md).

## Project status

Initialized; setup with the operator is not yet complete.
