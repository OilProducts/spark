---
type: tool
title: Prepare Workspace
extensions:
  tool.artifacts.stderr: prepare-workspace.stderr.txt
  tool.artifacts.stdout: prepare-workspace.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab shared prover refuter
    if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
      cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
    fi
    if [ ! -f DOSSIER.md ] && [ -f /opt/mathlab/workspace-template/DOSSIER.md ]; then
      cp /opt/mathlab/workspace-template/DOSSIER.md DOSSIER.md
    fi
    git init -q 2>/dev/null || true
routes:
- to: frame_problem
---

```sh
set -eu
mkdir -p .mathlab shared prover refuter
if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
  cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
fi
if [ ! -f DOSSIER.md ] && [ -f /opt/mathlab/workspace-template/DOSSIER.md ]; then
  cp /opt/mathlab/workspace-template/DOSSIER.md DOSSIER.md
fi
git init -q 2>/dev/null || true
```
