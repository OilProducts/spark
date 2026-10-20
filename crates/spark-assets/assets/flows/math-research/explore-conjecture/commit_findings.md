---
type: tool
title: Commit Findings
extensions:
  tool.artifacts.stderr: commit-findings.stderr.txt
  tool.artifacts.stdout: commit-findings.stdout.txt
  tool.command: |-
    set -eu
    if git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
      echo "Skipping commit: no repository changes detected."
      exit 0
    fi
    git add -A
    git commit -m "Explore: $(head -c 72 survey.md 2>/dev/null | head -1 || echo session)"
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
git commit -m "Explore: $(head -c 72 survey.md 2>/dev/null | head -1 || echo session)"
```
