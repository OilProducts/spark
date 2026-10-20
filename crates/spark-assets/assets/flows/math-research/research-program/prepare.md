---
type: tool
title: Prepare Workspace
extensions:
  tool.artifacts.stderr: prepare.stderr.txt
  tool.artifacts.stdout: prepare.stdout.txt
  tool.command: |-
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
routes:
- to: research
  condition: outcome=success
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
