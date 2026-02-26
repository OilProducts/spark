#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKLIST_FILE="${CHECKLIST_FILE:-$ROOT_DIR/attractor-implementation-checklist.md}"
MAX_ITERATIONS="${MAX_ITERATIONS:-100}"
STALL_LIMIT="${STALL_LIMIT:-3}"
CODEX_SANDBOX="${CODEX_SANDBOX:-danger-full-access}"

usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Options:
  --checklist PATH        Checklist file path (default: $CHECKLIST_FILE)
  --max-iterations N      Maximum loop iterations (default: $MAX_ITERATIONS)
  --stall-limit N         Stop after N non-progress iterations (default: $STALL_LIMIT)
  -h, --help              Show this help text

Environment overrides:
  CHECKLIST_FILE, MAX_ITERATIONS, STALL_LIMIT, CODEX_SANDBOX
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --checklist)
      CHECKLIST_FILE="$2"
      shift 2
      ;;
    --max-iterations)
      MAX_ITERATIONS="$2"
      shift 2
      ;;
    --stall-limit)
      STALL_LIMIT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v codex >/dev/null 2>&1; then
  echo "error: 'codex' CLI not found in PATH" >&2
  exit 1
fi

if [[ ! -f "$CHECKLIST_FILE" ]]; then
  echo "error: checklist file not found: $CHECKLIST_FILE" >&2
  exit 1
fi

count_unchecked() {
  rg -n '^\s*-\s*\[ \]' "$CHECKLIST_FILE" | wc -l | tr -d ' '
}

read -r -d '' PROMPT <<'EOF' || true
Use attractor-implementation-checklist.md and pick the next unchecked item.

Workflow:
1) Determine if the item is ready to be workd now by scanning code/tests/spec. If not ready, move it to a “Deferred Tasks” section at the end of the checklist and explain why in one sentence.
2) If ready, do strict test-first:
   - Add/update tests for only this item.
   - Run `just test` (use pytest, not unittest entrypoints).
   - Confirm red, then implement minimal code to turn green.
3) Keep scope narrow. No unrelated refactors.
4) After implementation, spawn a sub-agent to evaluate whether the item is tested and implemented in the spirit of attractor-spec.md. Include sub-agent verdict and evidence in your report.
5) Commit your changes with a clear message. If there are unrelated untracked files, do not include them.

Output:
- Selected item and readiness decision
- Test changes
- Code changes
- Commands run + outcomes
- Sub-agent verdict (verbatim)
- Difficulties and workflow improvements
- Commit hash
EOF

unchecked_before="$(count_unchecked)"
if [[ "$unchecked_before" -eq 0 ]]; then
  echo "Checklist already complete: $CHECKLIST_FILE"
  exit 0
fi

echo "Starting loop with $unchecked_before unchecked item(s)."
stalled_iterations=0

for ((i = 1; i <= MAX_ITERATIONS; i++)); do
  echo
  echo "=== Iteration $i ==="
  echo "Unchecked before run: $unchecked_before"

  codex exec --sandbox "$CODEX_SANDBOX" -C "$ROOT_DIR" "$PROMPT"

  unchecked_after="$(count_unchecked)"
  echo "Unchecked after run: $unchecked_after"

  if [[ "$unchecked_after" -eq 0 ]]; then
    echo "Checklist complete."
    exit 0
  fi

  if [[ "$unchecked_after" -ge "$unchecked_before" ]]; then
    stalled_iterations=$((stalled_iterations + 1))
    echo "No progress detected ($stalled_iterations/$STALL_LIMIT stall limit)."
  else
    stalled_iterations=0
  fi

  if [[ "$stalled_iterations" -ge "$STALL_LIMIT" ]]; then
    echo "Stopping: no checklist progress for $STALL_LIMIT consecutive iteration(s)." >&2
    exit 2
  fi

  unchecked_before="$unchecked_after"
done

echo "Stopping: reached MAX_ITERATIONS=$MAX_ITERATIONS with work still remaining." >&2
exit 3
