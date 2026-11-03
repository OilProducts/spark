---
type: tool
title: Prepare Workspace
config:
  exit_codes:
    '0': done
extensions:
  tool.artifacts.stderr: prepare-workspace.stderr.txt
  tool.artifacts.stdout: prepare-workspace.stdout.txt
routes:
  done:
    to: survey
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
mkdir -p .mathlab experiments
if [ ! -f AGENTS.md ] && [ -f /opt/mathlab/workspace-template/AGENTS.md ]; then
  cp /opt/mathlab/workspace-template/AGENTS.md AGENTS.md
fi
if [ ! -f DOSSIER.md ] && [ -f /opt/mathlab/workspace-template/DOSSIER.md ]; then
  cp /opt/mathlab/workspace-template/DOSSIER.md DOSSIER.md
fi
git init -q 2>/dev/null || true
```
