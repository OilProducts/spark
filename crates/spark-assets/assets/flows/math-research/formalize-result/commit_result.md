---
type: tool
title: Commit Result
config:
  exit_codes:
    '0': done
extensions:
  tool.artifacts.stderr: commit-result.stderr.txt
  tool.artifacts.stdout: commit-result.stdout.txt
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
git commit -m "Formalize: $(head -c 72 .mathlab/theorems.txt 2>/dev/null | head -1 || echo result)"
```
