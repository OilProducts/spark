---
type: tool
title: Commit Findings
config:
  exit_codes:
    '0': done
extensions:
  tool.artifacts.stderr: commit-findings.stderr.txt
  tool.artifacts.stdout: commit-findings.stdout.txt
routes:
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
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
