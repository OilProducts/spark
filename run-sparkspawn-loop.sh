#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKLIST_FILE="$ROOT_DIR/ui-implementation-checklist.md"
MAX_ITERATIONS="${MAX_ITERATIONS:-100}"
STALL_LIMIT="${STALL_LIMIT:-3}"
CODEX_SANDBOX="${CODEX_SANDBOX:-danger-full-access}"

usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Options:
  --max-iterations N      Maximum loop iterations (default: $MAX_ITERATIONS)
  --stall-limit N         Stop after N non-progress iterations (default: $STALL_LIMIT)
  -h, --help              Show this help text

Environment overrides:
  MAX_ITERATIONS, STALL_LIMIT, CODEX_SANDBOX
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
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
  local checklist_file="${1:-$CHECKLIST_FILE}"
  rg -n '^\s*-\s*\[ \]' "$checklist_file" | wc -l | tr -d ' '
}

count_unchecked_active() {
  local checklist_file="${1:-$CHECKLIST_FILE}"
  awk '
    BEGIN { in_deferred = 0; c = 0 }
    /^## Deferred Tasks/ { in_deferred = 1 }
    in_deferred == 0 && $0 ~ /^[[:space:]]*-[[:space:]]*\[ \]/ { c++ }
    END { print c + 0 }
  ' "$checklist_file"
}

count_unchecked_deferred() {
  local checklist_file="${1:-$CHECKLIST_FILE}"
  awk '
    BEGIN { in_deferred = 0; c = 0 }
    /^## Deferred Tasks/ { in_deferred = 1; next }
    in_deferred == 1 && $0 ~ /^[[:space:]]*-[[:space:]]*\[ \]/ { c++ }
    END { print c + 0 }
  ' "$checklist_file"
}

checklist_hash() {
  local checklist_file="${1:-$CHECKLIST_FILE}"
  shasum "$checklist_file" | awk '{print $1}'
}

extract_evaluator_verdict() {
  local log_file="$1"
  local verdict
  verdict="$(
    awk '
      function ltrim(s) { sub(/^[[:space:]]+/, "", s); return s }
      function rtrim(s) { sub(/[[:space:]]+$/, "", s); return s }
      function trim(s) { return rtrim(ltrim(s)) }
      {
        line = tolower($0)
        gsub(/\r/, "", line)
        gsub(/`/, "", line)
        gsub(/\*\*/, "", line)
        gsub(/^[[:space:]]*[-*][[:space:]]*/, "", line)
        if (line ~ /^evaluator_verdict[[:space:]]*:/) {
          value = line
          sub(/^evaluator_verdict[[:space:]]*:[[:space:]]*/, "", value)
          value = trim(value)
          if (index(value, "|") > 0) {
            next
          }
          sub(/[[:space:]].*$/, "", value)
          gsub(/[^a-z-].*$/, "", value)
          if (value != "") {
            parsed = value
          }
        }
      }
      END {
        if (parsed != "") {
          print parsed
        }
      }
    ' "$log_file"
  )"
  case "$verdict" in
    pass|fail|needs-human) ;;
    *) verdict="" ;;
  esac
  if [[ -z "${verdict:-}" ]]; then
    verdict="unknown"
  fi
  echo "$verdict"
}

extract_checklist_status_update() {
  local log_file="$1"
  local status
  status="$(
    awk '
      function ltrim(s) { sub(/^[[:space:]]+/, "", s); return s }
      function rtrim(s) { sub(/[[:space:]]+$/, "", s); return s }
      function trim(s) { return rtrim(ltrim(s)) }
      {
        line = tolower($0)
        gsub(/\r/, "", line)
        gsub(/`/, "", line)
        gsub(/\*\*/, "", line)
        gsub(/^[[:space:]]*[-*][[:space:]]*/, "", line)
        if (line ~ /^checklist_status_update[[:space:]]*:/) {
          value = line
          sub(/^checklist_status_update[[:space:]]*:[[:space:]]*/, "", value)
          value = trim(value)
          if (index(value, "|") > 0) {
            next
          }
          sub(/[[:space:]].*$/, "", value)
          gsub(/[^a-z-].*$/, "", value)
          if (value != "") {
            parsed = value
          }
        }
      }
      END {
        if (parsed != "") {
          print parsed
        }
      }
    ' "$log_file"
  )"
  case "$status" in
    checked|unchecked|deferred) ;;
    *) status="" ;;
  esac
  if [[ -z "${status:-}" ]]; then
    status="unknown"
  fi
  echo "$status"
}

extract_selected_item_id() {
  local log_file="$1"
  local selected
  selected="$(
    awk '
      function ltrim(s) { sub(/^[[:space:]]+/, "", s); return s }
      function rtrim(s) { sub(/[[:space:]]+$/, "", s); return s }
      function trim(s) { return rtrim(ltrim(s)) }
      {
        raw = $0
        line = tolower($0)
        gsub(/\r/, "", line)
        gsub(/`/, "", raw)
        gsub(/\*\*/, "", raw)
        if (line ~ /selected item|selected deferred item|selected:/) {
          if (match(raw, /\[[A-Za-z0-9][A-Za-z0-9._-]*\]/)) {
            value = substr(raw, RSTART + 1, RLENGTH - 2)
            value = trim(value)
            if (value != "") {
              parsed = value
            }
          }
        }
      }
      END {
        if (parsed != "") {
          print parsed
        }
      }
    ' "$log_file"
  )"
  if [[ -z "${selected:-}" ]]; then
    selected="unknown"
  fi
  echo "$selected"
}

worktree_progress_fingerprint() {
  local worktree_dir="$1"
  (
    git -C "$worktree_dir" status --porcelain=v1 --untracked-files=all
    git -C "$worktree_dir" diff
    git -C "$worktree_dir" diff --cached
  ) | shasum | awk '{print $1}'
}

report_stop_with_iteration_artifacts() {
  local exit_code="$1"
  local reason="$2"
  local iteration_log="$3"
  local iteration_worktree="${4:-}"

  echo "Stopping: $reason" >&2
  echo "Iteration log retained at: $iteration_log" >&2
  if [[ -n "$iteration_worktree" ]]; then
    echo "Iteration worktree retained at: $iteration_worktree" >&2
  fi
  exit "$exit_code"
}

create_iteration_worktree() {
  local base_head="$1"
  local iteration_worktree
  iteration_worktree="$(mktemp -d "${TMPDIR:-/tmp}/sparkspawn-loop-worktree.XXXXXX")"
  if ! git -C "$ROOT_DIR" worktree add --detach "$iteration_worktree" "$base_head" >/dev/null 2>&1; then
    rm -rf "$iteration_worktree"
    echo "error: failed to create iteration worktree at HEAD $base_head" >&2
    exit 1
  fi
  echo "$iteration_worktree"
}

cleanup_iteration_worktree() {
  local iteration_worktree="$1"
  git -C "$ROOT_DIR" worktree remove --force "$iteration_worktree" >/dev/null 2>&1 || rm -rf "$iteration_worktree"
}

read -r -d '' ACTIVE_PROMPT <<'EOF' || true
Use ui-implementation-checklist.md and pick the next unchecked item.

Workflow:
1) Determine if the item is ready to be worked now by scanning code/tests/spec. If not ready, move it to a “Deferred Tasks” section at the end of the checklist and explain why in one sentence.
2) If ready, do strict test-first:
   - Add/update tests for only this item.
   - Run `just test` (use pytest, not unittest entrypoints).
   - Confirm red, then implement minimal code to turn green.
   - If your changes touch `frontend/`, run `just ui-smoke` to generate screenshots in `frontend/artifacts/ui-smoke/`.
3) Keep scope narrow. No unrelated refactors.
4) Checklist gate rules (required):
   - Keep the selected checklist item unchecked (`[ ]`) until evaluator approval is known.
   - Only mark the item checked (`[x]`) if evaluator verdict is `pass`.
   - If evaluator verdict is `fail` or `needs-human`, keep item unchecked and add/update a one-sentence blocker note.
   - If you marked `[x]` before evaluation, revert it back to `[ ]` before commit unless verdict is `pass`.
5) After implementation, spawn a sub-agent to evaluate whether the item is tested and implemented in the spirit of ui-spec.md.
   - If your changes touched `frontend/`, you MUST provide screenshot paths from `frontend/artifacts/ui-smoke/` to the sub-agent and ask for visual QA findings from those images.
   - Include sub-agent verdict and evidence in your report.
6) Commit your changes with a clear message. If there are unrelated untracked files, do not include them.

Output:
- Selected item and readiness decision
- Test changes
- Code changes
- Commands run + outcomes
- If `frontend/` changed: UI smoke result and screenshot paths under `frontend/artifacts/ui-smoke/`
- Sub-agent verdict (verbatim). If `frontend/` changed, this verdict must include screenshot-based visual QA findings.
- EVALUATOR_VERDICT: pass | fail | needs-human (plain token, no backticks)
- CHECKLIST_STATUS_UPDATE: checked | unchecked | deferred (plain token, no backticks)
- Difficulties and workflow improvements
- Commit hash
EOF

read -r -d '' DEFERRED_PROMPT <<'EOF' || true
Use ui-implementation-checklist.md and reassess the next unchecked item under the “Deferred Tasks” section only.

Workflow:
1) Reassess readiness by scanning code/tests/spec.
2) Choose one action:
   - If already implemented/tested in spirit of ui-spec.md, mark that Deferred item as `[x]` with a short verification note.
   - If now ready and missing, do strict test-first for only that item (red -> minimal green), then mark it `[x]`.
   - If still blocked, keep it unchecked and tighten its one-sentence defer reason.
   - If your changes touch `frontend/`, run `just ui-smoke` to generate screenshots in `frontend/artifacts/ui-smoke/`.
3) Keep scope narrow. No unrelated refactors.
4) Checklist gate rules (required):
   - Keep the selected checklist item unchecked (`[ ]`) until evaluator approval is known.
   - Only mark the item checked (`[x]`) if evaluator verdict is `pass`.
   - If evaluator verdict is `fail` or `needs-human`, keep item unchecked and tighten the defer blocker reason.
   - If you marked `[x]` before evaluation, revert it back to `[ ]` before commit unless verdict is `pass`.
5) Spawn a sub-agent to evaluate whether your action is correct in spirit of ui-spec.md.
   - If your changes touched `frontend/`, you MUST provide screenshot paths from `frontend/artifacts/ui-smoke/` to the sub-agent and ask for visual QA findings from those images.
   - Include verdict and evidence.
6) Commit changes with a clear message. If there are unrelated untracked files, do not include them.

Output:
- Selected deferred item and reassessment decision
- Action taken (`verified-existing` / `implemented-now` / `still-blocked`)
- Test changes
- Code/checklist changes
- Commands run + outcomes
- If `frontend/` changed: UI smoke result and screenshot paths under `frontend/artifacts/ui-smoke/`
- Sub-agent verdict (verbatim). If `frontend/` changed, this verdict must include screenshot-based visual QA findings.
- EVALUATOR_VERDICT: pass | fail | needs-human (plain token, no backticks)
- CHECKLIST_STATUS_UPDATE: checked | unchecked | deferred (plain token, no backticks)
- Difficulties and workflow improvements
- Commit hash
EOF

active_before="$(count_unchecked_active)"
deferred_before="$(count_unchecked_deferred)"
total_before=$((active_before + deferred_before))
hash_before="$(checklist_hash)"

if [[ "$total_before" -eq 0 ]]; then
  echo "Checklist already complete: $CHECKLIST_FILE"
  exit 0
fi

echo "Starting loop: active_unchecked=$active_before deferred_unchecked=$deferred_before total_unchecked=$total_before"
stalled_iterations=0
active_iteration_worktree=""
active_iteration_base_head=""
active_item_id=""
active_iteration_mode=""

for ((i = 1; i <= MAX_ITERATIONS; i++)); do
  phase="active"
  prompt="$ACTIVE_PROMPT"
  if [[ -n "$active_iteration_worktree" ]]; then
    phase="continue"
    if [[ "$active_iteration_mode" == "deferred" ]]; then
      prompt="$DEFERRED_PROMPT"
    else
      prompt="$ACTIVE_PROMPT"
    fi
    continuation_target="current pinned item"
    if [[ -n "$active_item_id" ]]; then
      continuation_target="[$active_item_id]"
    fi
    prompt="$prompt

Continuation instructions:
- Continue working checklist item $continuation_target in this existing iteration workspace.
- Do not switch to a different checklist item.
- Preserve prior iteration progress and iterate only on this item until evaluator verdict is pass."
  elif [[ "$active_before" -eq 0 && "$deferred_before" -gt 0 ]]; then
    phase="deferred"
    prompt="$DEFERRED_PROMPT"
  fi

  echo
  echo "=== Iteration $i ==="
  echo "Phase: $phase"
  echo "Before run: active_unchecked=$active_before deferred_unchecked=$deferred_before total_unchecked=$total_before"

  iteration_log="$(mktemp)"
  root_head_before="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || true)"
  if [[ -n "$active_iteration_worktree" ]]; then
    iteration_worktree="$active_iteration_worktree"
    if [[ ! -d "$iteration_worktree" ]]; then
      report_stop_with_iteration_artifacts 13 "active iteration worktree is missing." "$iteration_log" "$iteration_worktree"
    fi
    if [[ "$root_head_before" != "$active_iteration_base_head" ]]; then
      report_stop_with_iteration_artifacts 14 "root HEAD changed while carrying an in-progress iteration worktree; refusing to continue." "$iteration_log" "$iteration_worktree"
    fi
    echo "Reusing in-progress iteration worktree: $iteration_worktree"
  else
    iteration_worktree="$(create_iteration_worktree "$root_head_before")"
    active_iteration_worktree="$iteration_worktree"
    active_iteration_base_head="$root_head_before"
    active_iteration_mode="$phase"
  fi
  iteration_checklist="$iteration_worktree/ui-implementation-checklist.md"
  if [[ ! -f "$iteration_checklist" ]]; then
    report_stop_with_iteration_artifacts 9 "iteration checklist not found in worktree." "$iteration_log" "$iteration_worktree"
  fi

  iteration_head_before="$(git -C "$iteration_worktree" rev-parse HEAD 2>/dev/null || true)"
  iteration_checklist_hash_before="$(checklist_hash "$iteration_checklist")"
  iteration_progress_before="$(worktree_progress_fingerprint "$iteration_worktree")"

  if codex exec --sandbox "$CODEX_SANDBOX" -C "$iteration_worktree" "$prompt" 2>&1 | tee "$iteration_log"; then
    codex_exit=0
  else
    codex_exit=$?
  fi
  iteration_head_after="$(git -C "$iteration_worktree" rev-parse HEAD 2>/dev/null || true)"

  if [[ "$codex_exit" -ne 0 ]]; then
    report_stop_with_iteration_artifacts 8 "codex exec failed." "$iteration_log" "$iteration_worktree"
  fi
  evaluator_verdict="$(extract_evaluator_verdict "$iteration_log")"
  checklist_status_update="$(extract_checklist_status_update "$iteration_log")"
  selected_item_id="$(extract_selected_item_id "$iteration_log")"
  if [[ "$selected_item_id" != "unknown" ]]; then
    if [[ -z "$active_item_id" ]]; then
      active_item_id="$selected_item_id"
      echo "Pinned checklist item: [$active_item_id]"
    elif [[ "$selected_item_id" != "$active_item_id" ]]; then
      report_stop_with_iteration_artifacts 15 "selected checklist item changed from [$active_item_id] to [$selected_item_id] inside an in-progress iteration." "$iteration_log" "$iteration_worktree"
    fi
  fi

  active_after_candidate="$(count_unchecked_active "$iteration_checklist")"
  deferred_after_candidate="$(count_unchecked_deferred "$iteration_checklist")"
  total_after_candidate=$((active_after_candidate + deferred_after_candidate))
  hash_after_candidate="$(checklist_hash "$iteration_checklist")"
  echo "After run: active_unchecked=$active_after_candidate deferred_unchecked=$deferred_after_candidate total_unchecked=$total_after_candidate"
  echo "Evaluator verdict: $evaluator_verdict"
  echo "Checklist status update: $checklist_status_update"

  if [[ "$evaluator_verdict" == "unknown" ]]; then
    report_stop_with_iteration_artifacts 6 "missing required machine-readable EVALUATOR_VERDICT output." "$iteration_log" "$iteration_worktree"
  fi

  if [[ "$checklist_status_update" == "unknown" ]]; then
    report_stop_with_iteration_artifacts 7 "missing required machine-readable CHECKLIST_STATUS_UPDATE output." "$iteration_log" "$iteration_worktree"
  fi

  if [[ "$total_after_candidate" -lt "$total_before" && "$evaluator_verdict" != "pass" ]]; then
    report_stop_with_iteration_artifacts 4 "checklist unchecked count decreased without evaluator pass verdict. Expected EVALUATOR_VERDICT: pass when marking an item complete." "$iteration_log" "$iteration_worktree"
  fi

  if [[ "$checklist_status_update" == "checked" && "$evaluator_verdict" != "pass" ]]; then
    report_stop_with_iteration_artifacts 5 "implementor reported CHECKLIST_STATUS_UPDATE=checked with non-pass evaluator verdict." "$iteration_log" "$iteration_worktree"
  fi

  iteration_progress_after="$(worktree_progress_fingerprint "$iteration_worktree")"
  worktree_progress=0
  if [[ "$iteration_head_after" != "$iteration_head_before" ]]; then
    worktree_progress=1
  elif [[ "$hash_after_candidate" != "$iteration_checklist_hash_before" ]]; then
    worktree_progress=1
  elif [[ "$iteration_progress_after" != "$iteration_progress_before" ]]; then
    worktree_progress=1
  fi

  if [[ "$evaluator_verdict" == "pass" ]]; then
    mapfile -t iteration_commits < <(git -C "$iteration_worktree" rev-list --reverse "${active_iteration_base_head}..${iteration_head_after}")
    if [[ "${#iteration_commits[@]}" -eq 0 ]]; then
      if [[ "$worktree_progress" -eq 1 ]]; then
        report_stop_with_iteration_artifacts 10 "iteration made progress but produced no commits; cannot import pass verdict result." "$iteration_log" "$iteration_worktree"
      fi
    else
      root_head_now="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || true)"
      if [[ "$root_head_now" != "$active_iteration_base_head" ]]; then
        report_stop_with_iteration_artifacts 11 "repository HEAD moved during in-progress item; refusing to cherry-pick iteration commits." "$iteration_log" "$iteration_worktree"
      fi

      for commit in "${iteration_commits[@]}"; do
        if ! git -C "$ROOT_DIR" cherry-pick "$commit" >/dev/null 2>&1; then
          git -C "$ROOT_DIR" cherry-pick --abort >/dev/null 2>&1 || true
          report_stop_with_iteration_artifacts 12 "failed to cherry-pick iteration commit $commit into root worktree." "$iteration_log" "$iteration_worktree"
        fi
      done
    fi

    active_after="$(count_unchecked_active "$CHECKLIST_FILE")"
    deferred_after="$(count_unchecked_deferred "$CHECKLIST_FILE")"
    total_after=$((active_after + deferred_after))
    hash_after="$(checklist_hash "$CHECKLIST_FILE")"

    cleanup_iteration_worktree "$iteration_worktree"
    active_iteration_worktree=""
    active_iteration_base_head=""
    active_item_id=""
    active_iteration_mode=""
  else
    active_after="$active_before"
    deferred_after="$deferred_before"
    total_after="$total_before"
    hash_after="$hash_before"
    if [[ -n "$active_item_id" ]]; then
      echo "Retaining in-progress worktree for [$active_item_id]: $iteration_worktree"
    else
      echo "Retaining in-progress worktree: $iteration_worktree"
    fi
  fi

  rm -f "$iteration_log"

  if [[ "$total_after" -eq 0 ]]; then
    echo "Checklist complete."
    exit 0
  fi

  made_progress=0
  if [[ "$evaluator_verdict" == "pass" ]]; then
    if [[ "$total_after" -lt "$total_before" ]]; then
      made_progress=1
    elif [[ "$hash_after" != "$hash_before" ]]; then
      # Deferred reassessment can legitimately improve notes/actionability
      # without reducing unchecked count immediately.
      made_progress=1
    fi
  elif [[ "$worktree_progress" -eq 1 ]]; then
    made_progress=1
  fi

  if [[ "$made_progress" -eq 0 ]]; then
    stalled_iterations=$((stalled_iterations + 1))
    echo "No progress detected ($stalled_iterations/$STALL_LIMIT stall limit)."
  else
    stalled_iterations=0
  fi

  if [[ "$stalled_iterations" -ge "$STALL_LIMIT" ]]; then
    echo "Stopping: no checklist progress for $STALL_LIMIT consecutive iteration(s)." >&2
    exit 2
  fi

  active_before="$active_after"
  deferred_before="$deferred_after"
  total_before="$total_after"
  hash_before="$hash_after"
done

echo "Stopping: reached MAX_ITERATIONS=$MAX_ITERATIONS with work still remaining." >&2
exit 3
