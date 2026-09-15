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
`container.mounts` paths. The source is a dedicated directory for this profile;
the destination must match the worker's effective `codex_runtime_root` (the
example uses `/home/YOU/.spark/runtime/codex`). Set that absolute runtime root
in Spark's Agent session limits if needed so native and container workers agree
on the destination. The mounted source keeps a separate login between containers.
Sign in once using that source directory:

```sh
docker run --rm -it \
  -v /home/YOU/.spark-mathlab-codex:/codex-runtime \
  -e CODEX_HOME=/codex-runtime/.codex \
  spark-mathlab:latest codex login --device-auth
```

This execution container has its own runtime, separate from the Spark server's
connection in Settings. Replace older `/mnt/codex-auth` mounts with the new
runtime mount and sign in afresh; never copy a host `auth.json`. Credentials
stay outside image layers, and Codex can persist token refreshes.

## Use

Launch a flow with `execution_profile_id: "math-lab"`. The flows under
`flows/math-research/` are designed for this profile and seed empty
project directories from `/opt/mathlab/workspace-template` (integrity
rules in its AGENTS.md; problem and current-state memory templates)
and `/opt/mathlab/template` (a Lean project wired to the image's prebuilt
Mathlib; pinned commit recorded in `/opt/mathlab/MATHLIB_COMMIT`).

Programs on the same problem should reuse one project directory. The research
program keeps the exact question in `problem.md`, the concise current claim
state in `state.md`, immutable action records under `history/`, and supporting
artifacts under `evidence/`.
