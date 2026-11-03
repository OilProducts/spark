---
type: tool
title: Prepare Workspace
config:
  exit_codes:
    '0': done
extensions:
  tool.artifacts.stderr: prepare.stderr.txt
  tool.artifacts.stdout: prepare.stdout.txt
routes:
  done:
    to: research
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
mkdir -p .math-research history evidence
rm -f .math-research/current-action .math-research/repair-used
if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
  cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
fi
if [ ! -f problem.md ] && [ -f /opt/mathlab/workspace-template/problem.md ]; then
  cp /opt/mathlab/workspace-template/problem.md problem.md
fi
if [ ! -f state.md ] && [ -f /opt/mathlab/workspace-template/state.md ]; then
  cp /opt/mathlab/workspace-template/state.md state.md
fi
touch problem.md state.md
git init -q 2>/dev/null || true
```
