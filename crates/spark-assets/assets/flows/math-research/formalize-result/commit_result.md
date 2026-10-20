---
type: tool
title: Commit Result
extensions:
  tool.artifacts.stderr: commit-result.stderr.txt
  tool.artifacts.stdout: commit-result.stdout.txt
  tool.command: |-
    set -eu
    if git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
      echo "Skipping commit: no repository changes detected."
      exit 0
    fi
    git add -A
    git commit -m "Formalize: $(head -c 72 .mathlab/theorems.txt 2>/dev/null | head -1 || echo result)"
routes:
- to: done
---

```sh
set -eu
if git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
  echo "Skipping commit: no repository changes detected."
  exit 0
fi
git add -A
git commit -m "Formalize: $(head -c 72 .mathlab/theorems.txt 2>/dev/null | head -1 || echo result)"
```
