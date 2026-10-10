// Real data from run-18d93c056079b9c0 (software-development/implement-change.yaml,
// CR-2026-0127), read from the run record on 2026-09-28. Times are local (EDT).
// The neighbouring runs in RUNS are illustrative.

const RUN = {
  id: 'run-18d93c056079b9c0',
  flow: 'software-development/implement-change.yaml',
  flowLabel: 'Implement change',
  objective: 'CR-2026-0127 · Provider-native reasoning effort',
  artifact: 'changes/CR-2026-0127-provider-native-reasoning-effort/request.md',
  project: 'spark',
  status: 'completed',
  started: '12:58:40',
  ended: '14:05:54',
  model: 'gpt-6-astra',
  tokens: 5517077,
  childTokens: 17059322,
  branch: 'spark/implement-change/run-18d93c056079b9c0',
  commit: '6debdc97bfbdf337d60c301d4df6730b1d040262',
  result: [
    ['Completed successfully', 'implemented CR-2026-0127, provider-native reasoning effort phase 1.'],
    ['Validation', 'final just test passed: Rust suites, formatting, 617 frontend tests, TypeScript checks, production build.'],
    ['Issues resolved', 'review caught unwanted lowercasing of effort values and Anthropic discovery ignoring configured credentials/endpoints. Earlier validation hit intermittent socket failures; two test helpers were stabilized.'],
  ],
}

// Flow graph (node id -> label, kind) and edges, from the run's flow snapshot.
const NODES = [
  { id: 'start', label: 'Start', kind: 'start' },
  { id: 'normalize_task', label: 'Normalize Task', kind: 'agent' },
  { id: 'prepare_workspace', label: 'Prepare Workspace', kind: 'tool' },
  { id: 'implement', label: 'Implement', kind: 'subflow' },
  { id: 'evaluate', label: 'Evaluate', kind: 'agent' },
  { id: 'validate', label: 'Validate', kind: 'tool' },
  { id: 'count_validate_attempt', label: 'Count Validate Attempt', kind: 'tool' },
  { id: 'diagnose_validation', label: 'Diagnose Validation', kind: 'agent' },
  { id: 'finalize_commit', label: 'Finalize Commit', kind: 'tool' },
  { id: 'cleanup_workspace', label: 'Cleanup Workspace', kind: 'tool' },
  { id: 'done', label: 'Done', kind: 'exit' },
]
const EDGES = [
  ['start', 'normalize_task'], ['normalize_task', 'prepare_workspace'], ['prepare_workspace', 'implement'],
  ['implement', 'evaluate'], ['evaluate', 'validate', 'Ready'], ['evaluate', 'implement', 'Fix'],
  ['validate', 'finalize_commit'], ['validate', 'count_validate_attempt', 'Fix'],
  ['count_validate_attempt', 'diagnose_validation'], ['diagnose_validation', 'implement', 'Fix'],
  ['finalize_commit', 'cleanup_workspace'], ['cleanup_workspace', 'done'],
]

// One entry per node execution (stage index). outcome: success | fail (routed) | error.
const EXECUTIONS = [
  { i: 0, node: 'start', s: '12:58:40', e: '12:58:40', outcome: 'success' },
  { i: 1, node: 'normalize_task', s: '12:58:40', e: '12:59:06', outcome: 'success', summary: 'Normalized the change request into objective, acceptance criteria and validation command (just test).' },
  { i: 2, node: 'prepare_workspace', s: '12:59:06', e: '12:59:06', outcome: 'success', summary: 'Created isolated worktree on spark/implement-change/run-18d93c05 from da8b7666.' },
  { i: 3, node: 'implement', s: '12:59:06', e: '13:24:27', outcome: 'success', child: { id: 'cda3c29d', flow: 'Implement Task', tokens: 12254944, s: '12:59:06', e: '13:23:43' }, summary: 'Implemented CR-2026-0127 and completed result.md. just test passed, including all Rust suites, 617 frontend tests, and the production build.' },
  { i: 4, node: 'evaluate', s: '13:24:27', e: '13:28:02', outcome: 'fail', route: 'Fix', summary: 'Changes requested: real chat/workflow paths lowercase effort strings before provider dispatch, so unlisted values are not forwarded unchanged. just test failed in mission_routes_enforce_records… (passed when rerun alone).' },
  { i: 5, node: 'implement', s: '13:28:02', e: '13:38:39', outcome: 'success', child: { id: '243510de', flow: 'Implement Task', tokens: 3426621, s: '13:28:02', e: '13:37:56' }, summary: 'Preserved mixed-case API effort through chat, workflow, and CLI paths while retaining Codex and Claude Code behavior. Added execution-to-provider regression coverage. Final just test passed. Updated result.md.' },
  { i: 6, node: 'evaluate', s: '13:38:39', e: '13:42:20', outcome: 'success', route: 'Ready', summary: 'Approved. Verified against request.md; just test passed: formatting, Rust suites, 617 frontend tests.' },
  { i: 7, node: 'validate', s: '13:42:20', e: '13:44:08', outcome: 'error', route: 'Fix', summary: 'just test exited with code 1.' },
  { i: 8, node: 'count_validate_attempt', s: '13:44:08', e: '13:44:08', outcome: 'success', summary: 'attempts = 1' },
  { i: 9, node: 'diagnose_validation', s: '13:44:08', e: '13:45:23', outcome: 'success', route: 'Fix', summary: 'Flaky harness: workflow_executes_with_captured_provider… panicked on socket WouldBlock (os error 35); passed alone in 0.26s. Fix: set_nonblocking(false) after accept.' },
  { i: 10, node: 'implement', s: '13:45:23', e: '13:49:49', outcome: 'success', child: { id: '1e0bf58a', flow: 'Implement Task', tokens: 537119, s: '13:45:23', e: '13:49:16' }, summary: 'Applied the required blocking-socket fix, preserving deadlines and assertions. Both captured-connection tests and just test passed. No provider implementation changes were made for this harness race.' },
  { i: 11, node: 'evaluate', s: '13:49:49', e: '13:53:25', outcome: 'fail', route: 'Fix', summary: 'Changes requested: crates/spark-workspace/src/models.rs:79 reads only process env, ignoring Anthropic endpoint and credentials in spark.toml. Separately settings_execution.rs:303 failed with WouldBlock.' },
  { i: 12, node: 'implement', s: '13:53:25', e: '13:59:27', outcome: 'success', child: { id: 'fadfb2ef', flow: 'Implement Task', tokens: 841638, s: '13:53:26', e: '13:58:40' }, summary: 'Addressed all review findings. Anthropic discovery now uses resolved provider configuration; added deterministic configuration/cache coverage and fixed captured-chat socket blocking mode. Full just test passed.' },
  { i: 13, node: 'evaluate', s: '13:59:27', e: '14:03:14', outcome: 'success', route: 'Ready', summary: 'Approved. All Rust suites, 617 frontend tests, formatting and build passed. No blocking findings.' },
  { i: 14, node: 'validate', s: '14:03:14', e: '14:05:43', outcome: 'success', summary: 'just test passed.' },
  { i: 15, node: 'finalize_commit', s: '14:05:43', e: '14:05:44', outcome: 'success', summary: 'Committed 6debdc97.' },
  { i: 16, node: 'cleanup_workspace', s: '14:05:44', e: '14:05:54', outcome: 'success', summary: 'Removed worktree.' },
  { i: 17, node: 'done', s: '14:05:54', e: '14:05:54', outcome: 'success' },
]

// Loop passes: consecutive executions starting at each Implement.
const PASSES = [
  { title: 'Setup', from: 0, to: 2, verdict: null },
  { title: 'Pass 1', from: 3, to: 4, verdict: 'Review requested changes' },
  { title: 'Pass 2', from: 5, to: 9, verdict: 'Approved, then validation failed (flaky test)' },
  { title: 'Pass 3', from: 10, to: 11, verdict: 'Review requested changes' },
  { title: 'Pass 4', from: 12, to: 14, verdict: 'Approved and validated' },
  { title: 'Wrap-up', from: 15, to: 17, verdict: null },
]

// Transcript of evaluate execution #11 and child fadfb2ef (real, trimmed).
const TRANSCRIPTS = {
  11: [
    ['msg', 'I’ll inspect the contract, implementation diff, and validation evidence in the isolated worktree, without changing files.'],
    ['tool', 'git status --short; git diff --stat; cat changes/CR-2026-0127…/request.md'],
    ['tool', 'git diff -- crates/unified-llm-adapter/src crates/spark-workspace/src'],
    ['think', 'Reading required docs'],
    ['msg', 'The native request fields, profile declarations, discovery cache, and picker metadata are connected to the execution path. Checking configuration sources next.'],
    ['tool', "rg -n 'from_process_env|ProviderEnvironment|core.*provider' crates/"],
    ['think', 'Checking client configuration'],
    ['tool', 'just test > /tmp/spark-cr-0127-review-gate.log 2>&1'],
    ['msg', 'Blocking: models.rs:79 reads only the process environment, so Anthropic endpoint and credentials configured in spark.toml are ignored by discovery.'],
  ],
  12: [
    ['msg', 'I’ll inspect the discovery configuration path and test harness, address the review findings, then run the required checks and update the result.'],
    ['tool', 'git status --short; cat changes/CR-2026-0127…/request.md'],
    ['tool', "rg -n 'read_execution_configuration|execution_environment|ProviderEnvironment' crates/"],
    ['think', 'Updating cache test setup'],
    ['msg', 'Discovery currently bypasses the provider settings resolver used by execution. I’ll use that resolver and its credential mapping, then test it.'],
    ['tool', "python3 - <<'PY' … crates/spark-workspace/src/models.rs"],
    ['tool', 'cat >> crates/spark-workspace/tests/contracts/project_registry_contracts.rs'],
    ['tool', 'git diff -- crates/spark-workspace/src/models.rs crates/spark-workspace/tests/settings_execution.rs'],
  ],
}

const RUNS = [
  { id: 'run-18d999991f64', title: 'CR-2026-0131 · Triggers deliver events to missions', flow: 'Implement change', status: 'running', when: 'now', dur: '27m', node: 'evaluate (2nd)', project: 'spark' },
  { id: 'run-18d9f2a10b77', title: 'Mission: tighten run hand-offs — answer questions', flow: 'Question gate', status: 'waiting', when: '12m ago', dur: '4m', node: 'ask_user', project: 'spark' },
  { id: 'run-18d9543bcf14', title: 'CR-2026-0130 · Lock run-record rewrites', flow: 'Implement change', status: 'completed', when: '2h ago', dur: '37m', passes: 2, project: 'spark' },
  { id: 'run-18d93c056079', title: 'CR-2026-0127 · Provider-native reasoning effort', flow: 'Implement change', status: 'completed', when: 'Yesterday 12:58', dur: '1h 7m', passes: 4, project: 'spark', current: true },
  { id: 'run-18d92e0b0bfd', title: 'CR-2026-0126 · Mission edits as agent turns', flow: 'Implement change', status: 'completed', when: 'Yesterday 10:02', dur: '56m', passes: 2, project: 'spark' },
  { id: 'run-18d86d7c2aa0', title: 'CR-2026-0121 · Missions replace tasks', flow: 'Implement change request', status: 'canceled', when: 'Sep 24', dur: '18m', project: 'spark' },
  { id: 'run-18d81a398250', title: 'CR-2026-0119 · Webhook dispatch release gate', flow: 'Implement change request', status: 'failed', when: 'Sep 23', dur: '42m', node: 'validate', project: 'spark' },
  { id: 'run-18c4a0b2d3e1', title: 'Prove/refute: Lemma 4.2 bound', flow: 'Prove / refute', status: 'failed', when: 'Sep 12', dur: '1h 7m', node: 'formalize', project: 'math-tuza' },
]

// ---- helpers ----
const toSec = (t) => { const [h, m, s] = t.split(':').map(Number); return h * 3600 + m * 60 + s }
const T0 = toSec(RUN.started)
const offset = (t) => toSec(t) - T0
const dur = (s, e) => {
  const d = toSec(e) - toSec(s)
  if (d < 1) return '<1s'
  if (d < 60) return `${d}s`
  const m = Math.floor(d / 60), r = d % 60
  if (m < 60) return r ? `${m}m ${r}s` : `${m}m`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}
const nodeById = Object.fromEntries(NODES.map((n) => [n.id, n]))
const visitsOf = (node) => EXECUTIONS.filter((x) => x.node === node)
const visitNo = (x) => visitsOf(x.node).indexOf(x) + 1
const visitLabel = (x) => { const n = visitsOf(x.node).length; return n > 1 ? `${visitNo(x)} of ${n}` : '' }
const fmtTok = (n) => n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : `${n}`
const OUTCOME = {
  success: { cls: 'ok', label: 'Succeeded' },
  fail: { cls: 'warn', label: 'Changes requested' },
  error: { cls: 'err', label: 'Failed' },
  running: { cls: 'run', label: 'Running' },
}
const outcomeLabel = (x) => x.node === 'validate' && x.outcome === 'error' ? 'Exit 1' : OUTCOME[x.outcome].label
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]))

function renderTranscript(i) {
  const rows = TRANSCRIPTS[i]
  if (!rows) return '<p class="muted small">Transcript loads on demand from <code>executions/' + i + '-0/transcript.jsonl</code>.</p>'
  return '<div class="transcript">' + rows.map(([k, t]) => k === 'tool'
    ? `<div class="tr-tool"><span class="chev">›</span><span class="tag">Run</span><code>${esc(t)}</code></div>`
    : k === 'think' ? `<div class="tr-think">Thought: ${esc(t)}</div>`
    : `<div class="tr-msg">${esc(t)}</div>`).join('') + '</div>'
}

function renderNav() {
  return `<header class="appnav"><div class="brand"><span class="logo"></span>Spark</div>
  <nav><a>Home</a><a>Missions</a><a>Editor</a><a class="on">Runs</a><a>Triggers</a></nav>
  <div class="navr"><span class="projchip">spark ▾</span></div></header>`
}

function runsListHtml({ compact = false } = {}) {
  const groups = [
    ['Needs you', RUNS.filter((r) => r.status === 'waiting')],
    ['Running', RUNS.filter((r) => r.status === 'running')],
    ['Recent', RUNS.filter((r) => !['waiting', 'running'].includes(r.status))],
  ]
  return `<div class="rl-search"><input placeholder="Search runs, CRs, flows…  ⌘K"/></div>
  <div class="rl-filters"><span class="chip on">This project</span><span class="chip">All</span><span class="chip">Failed</span></div>` +
  groups.filter(([, rs]) => rs.length).map(([g, rs]) => `<div class="rl-group">${g}<span>${rs.length}</span></div>` + rs.map((r) => `
    <div class="rl-row ${r.current ? 'sel' : ''}">
      <span class="dot ${r.status}"></span>
      <div class="rl-main"><div class="rl-title">${esc(r.title)}</div>
      <div class="rl-meta">${r.flow} · ${r.when} · ${r.dur}${r.passes ? ` · ${r.passes} passes` : ''}${r.node && r.status !== 'completed' ? ` · at ${r.node}` : ''}</div></div>
    </div>`).join('')).join('')
}

// Per-visit inputs/outputs, from each execution's prompt.md ("Declared context
// reads") and status.json context_updates. null = key cleared. Values trimmed.
const REVIEW_READS = ['task.objective', 'task.validation_command', 'workspace.path']
const IO = {
  1: { instr: 'Normalize the launch request into one bounded task. Treat exactly one of context.request.objective or context.request.artifact_path as the task source; an approved change-request document such as changes/<CR-id>/request.md is a valid artifact source whose requirements are the contract…', reads: ['request.acceptance_criteria', 'request.artifact_path', 'request.base_ref', 'request.constraints', 'request.objective', 'request.target_paths', 'request.validation_command'],
       writes: { 'task.objective': 'Implement provider-native reasoning effort phase 1 according to changes/CR-2026-0127…/request.md…', 'task.artifact_path': 'changes/CR-2026-0127-provider-native-reasoning-effort/request.md', 'task.validation_command': 'just test', 'task.base_ref': 'HEAD' } },
  2: { tool: 'prepare workspace (git worktree add)', exit: 0, output: '{"path":".spark/checkouts/run-18d93c056079b9c0","branch":"spark/implement-change/run-18d93c056079b9c0","base_commit":"da8b7666"}',
       writes: { 'workspace.path': '.spark/checkouts/run-18d93c056079b9c0', 'workspace.branch': 'spark/implement-change/run-18d93c056079b9c0', 'workspace.base_commit': 'da8b7666', 'workspace.source_dirty': 'true' } },
  4: { instr: 'You are the implementation gatekeeper. The implementation attempt lives in the isolated worktree at context.workspace.path; judge that repository state, not the implementer’s claims…', reads: REVIEW_READS,
       writes: { 'review.summary': 'Blocking contract gap: real chat/workflow paths lowercase effort strings before provider dispatch, so unlisted values are not forwarded unchanged…', 'review.required_changes': 'Preserve unlisted API-provider effort strings through the actual execution paths. Inspect lowercasing in spark-agent-adapter/src/llm_backend.rs:352,541,1203…' } },
  6: { instr: 'You are the implementation gatekeeper…', reads: REVIEW_READS, writes: { 'review.summary': null, 'review.required_changes': null } },
  7: { tool: 'just test', exit: 1, output: 'failures:\n    mission_contracts::cancel_stops_owned_runs_and_closes_the_mission\n\ntest result: FAILED. 62 passed; 1 failed; 1 ignored; finished in 7.70s\n\nerror: test failed, to rerun pass `-p spark-workspace --test contracts`\nerror: recipe `test` failed on line 48 with exit code 101',
       writes: { 'tool.exit_code': '1', 'tool.output': '…test result: FAILED. 62 passed; 1 failed…' } },
  8: { tool: 'count validate attempt', exit: 0, output: '{"attempts":1}', writes: { 'validation.attempts': '1' } },
  9: { instr: 'The deterministic validation gate just failed for the implementation in the isolated worktree at context.workspace.path. context.task.validation_command is the gate command, context.tool.output holds the tail of its log, and context.tool.exit_code its exit code. Your job is to translate that failure…', reads: ['task.objective', 'task.validation_command', 'tool.exit_code', 'tool.output', 'workspace.path'],
       writes: { 'review.summary': 'Supplied metadata contained exit_code=0 and no failure log. Running just test reproduced exit 101: workflow_executes_with_captured_provider_after_core_file_changes panicked on socket read WouldBlock…', 'review.required_changes': 'In pipeline_lifecycle_contracts.rs, stabilize workflow_executes_with_captured_connection: call stream.set_nonblocking(false) immediately after accepting…' } },
  11: { instr: 'You are the implementation gatekeeper. The implementation attempt lives in the isolated worktree at context.workspace.path; judge that repository state, not the implementer’s claims, and treat summaries as untrusted until verified…', reads: REVIEW_READS,
        writes: { 'review.summary': 'Blocking discovery configuration mismatch found by direct code-path inspection. crates/spark-workspace/src/models.rs:79 reads only process environment…', 'review.required_changes': '1. Make Anthropic discovery use the existing resolved provider configuration from spark.toml… 2. Fix the captured-chat test socket setup… 3. Update result.md…' } },
  13: { instr: 'You are the implementation gatekeeper…', reads: REVIEW_READS, writes: { 'review.summary': null, 'review.required_changes': null } },
  14: { tool: 'just test', exit: 0, output: '…\ndist/assets/index-QO2x.js   1,412.80 kB │ gzip: 402.11 kB\n✓ built in 9.84s', writes: { 'tool.exit_code': '0', 'tool.output': '…✓ built in 9.84s' } },
  15: { tool: 'finalize commit', exit: 0, output: '{"commit":"6debdc97bfbdf337d60c301d4df6730b1d040262"}', writes: { 'workspace.commit': '6debdc97' } },
  16: { tool: 'cleanup workspace', exit: 0, output: '', writes: {} },
}
for (const i of [3, 5, 10, 12]) IO[i] = { child: true, writes: {}, systemWrites: i === 3 ? 21 : 6 }
IO[9].writesNote = 'Diagnose reuses the review keys, so its finding reaches Implement the same way a review does.'

// Run-level context: the meaningful keys, grouped. System keys are counted, not listed.
const CONTEXT_GROUPS = [
  ['Task', ['task.objective', 'task.artifact_path', 'task.validation_command', 'task.base_ref']],
  ['Workspace', ['workspace.path', 'workspace.branch', 'workspace.base_commit', 'workspace.commit', 'workspace.source_dirty']],
  ['Review', ['review.summary', 'review.required_changes']],
  ['Validation', ['validation.attempts', 'tool.exit_code', 'tool.output']],
  ['Request', ['request.artifact_path', 'request.validation_command']],
]
const CONTEXT_REQUEST = { 'request.artifact_path': 'changes/CR-2026-0127-provider-native-reasoning-effort/request.md', 'request.validation_command': 'just test' }
const SYSTEM_KEY_COUNT = 70
