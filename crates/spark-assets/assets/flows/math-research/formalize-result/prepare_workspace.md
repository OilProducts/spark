---
type: tool
title: Prepare Workspace
extensions:
  tool.artifacts.stderr: prepare-workspace.stderr.txt
  tool.artifacts.stdout: prepare-workspace.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
      cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
    fi
    if [ ! -f DOSSIER.md ] && [ -f /opt/mathlab/workspace-template/DOSSIER.md ]; then
      cp /opt/mathlab/workspace-template/DOSSIER.md DOSSIER.md
    fi
    if [ ! -d lean ]; then
      cp -r /opt/mathlab/template lean
    fi
    git init -q 2>/dev/null || true
routes:
- to: formalize
---

```sh
set -eu
mkdir -p .mathlab
if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
  cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
fi
if [ ! -f DOSSIER.md ] && [ -f /opt/mathlab/workspace-template/DOSSIER.md ]; then
  cp /opt/mathlab/workspace-template/DOSSIER.md DOSSIER.md
fi
if [ ! -d lean ]; then
  cp -r /opt/mathlab/template lean
fi
git init -q 2>/dev/null || true
```
