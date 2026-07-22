# Math Lab execution container

A `local_container` execution environment for mathematical research flows:
Lean 4 with a prebuilt, pinned Mathlib as the proof gatekeeper, plus the
experimental-math stack (CaDiCaL, kissat, drat-trim, Z3, cvc5, nauty,
PARI/GP, CBC, CSDP, and a Python venv with sympy/networkx/python-sat/
z3-solver/cypari2/highspy), the codex CLI, and the spark worker entrypoint.

## Build

    containers/mathlab/build.sh

Builds `spark-mathlab:latest` (~13 GB; the Mathlib cache pull dominates)
from the repo root context. The worker binary compiles inside Docker for
the image architecture, so the same build works on Linux and macOS
(Docker Desktop) hosts, including Apple Silicon.

## Register the profile

Merge `execution-profiles.example.toml` into
`$SPARK_HOME/config/execution-profiles.toml`, adjusting the
`container.mounts` host path. Codex auth is bind-mounted read-only at
`/mnt/codex-auth` and seeded into a writable container-local home by
`mathlab-entry.sh` at container start — it is never baked into image
layers.

## Use

Launch a flow with `execution_profile_id: "math-lab"`. The flows under
`flows/math-research/` are designed for this profile and seed empty
project directories from `/opt/mathlab/workspace-template` (integrity
rules in its AGENTS.md; cross-session memory template in its DOSSIER.md)
and `/opt/mathlab/template` (a Lean project wired to the image's prebuilt
Mathlib; pinned commit recorded in `/opt/mathlab/MATHLIB_COMMIT`).

Sessions on the same problem should reuse one project directory: every
math-research flow reads the workspace's `DOSSIER.md` before working and
appends a session entry before committing, so results, failed attacks,
and repaired statements accumulate across runs.
