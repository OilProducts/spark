import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { useLiveThinkingStore } from '@/components/app/transcript/liveThinking'
import { useState, type ComponentProps } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type { PipelineResultResponse } from '@/lib/attractorClient'
import type { RunRecord, TimelineEventEntry } from '../../model/shared'
import { buildRunContextOverview, runStatusKind } from '../../model/runOverviewModel'
import { visitKey, type RunVisit, type VisitFlowNode, type VisitOutcome } from '../../model/visitModel'
import { RunContextItem } from '../RunContextItem'
import { RunStatusItem } from '../RunStatusItem'
import { RunVisitsCard } from '../RunVisitsCard'

const run = (overrides: Partial<RunRecord> = {}): RunRecord => ({
  run_id: 'run-1',
  flow_name: 'implement-change.yaml',
  status: 'completed',
  working_directory: '/tmp/project',
  model: 'gpt-5.4',
  started_at: '2026-09-29T10:00:00Z',
  ended_at: '2026-09-29T10:14:00Z',
  git_branch: 'spark/run-1',
  git_commit: 'abcdef0123',
  ...overrides,
})

const visit = (nodeId: string, number: number, outcome: VisitOutcome, extra: Partial<RunVisit> = {}): RunVisit => ({
  key: `${nodeId}-${number}`,
  runId: 'run-1',
  nodeId,
  stageIndex: number,
  attempt: 0,
  label: nodeId[0].toUpperCase() + nodeId.slice(1),
  kind: 'agent_task',
  number: 1,
  count: 1,
  startedAt: null,
  endedAt: null,
  outcome,
  decision: null,
  failureKind: null,
  reason: null,
  status: null,
  next: null,
  childRun: null,
  parentKey: null,
  ...extra,
})

const result: PipelineResultResponse = {
  run_id: 'run-1',
  status: 'completed',
  state: 'ready',
  source_node_id: 'commit',
  source_artifact_path: 'logs/commit/response.md',
  display_mode: 'raw',
  body_markdown: 'Committed the change.',
  summary_enabled: false,
}

const renderStatus = (record: RunRecord, visits: RunVisit[], hasQuestion = false) => {
  const onSelectVisit = vi.fn()
  const onViewArtifact = vi.fn()
  render(
    <RunStatusItem
      run={record}
      kind={runStatusKind(record.status, hasQuestion)}
      now={Date.parse('2026-09-29T11:00:00Z')}
      visits={visits}
      flowNodes={{ commit: { label: 'Commit Findings', kind: 'tool', readsContext: [], command: null } }}
      result={record.status === 'completed' ? result : null}
      resultError={null}
      artifactEntries={[
        { path: 'result/result.md', size_bytes: 10, media_type: 'text/markdown', viewable: true },
        { path: 'artifacts/flow/files.json', size_bytes: 10, media_type: 'application/json', viewable: true },
        { path: 'artifacts/flow/flow-definition.json', size_bytes: 10, media_type: 'application/json', viewable: true },
      ]}
      question={<p>Approve the change?</p>}
      onSelectVisit={onSelectVisit}
      onViewArtifact={onViewArtifact}
    />,
  )
  return { onSelectVisit, onViewArtifact }
}

describe('RunStatusItem', () => {
  it('shows a completed run result, its source, the errors it continued past, and its outputs', () => {
    const review = visit('review', 1, 'error', {
      reason: 'Missing a test.',
      failureKind: 'runtime',
      status: { status: 'error', failure_kind: 'runtime', tolerable: true, context_updates: { 'context.review.required_changes': 'Add a test.', 'internal.retry': 1 } },
    })
    const { onSelectVisit, onViewArtifact } = renderStatus(
      run({ outcome: 'success', outcome_reason_code: 'completed', outcome_reason_message: 'commit chose done' }),
      [review, visit('commit', 2, 'completed', { decision: 'done' })],
    )

    expect(screen.getByTestId('run-result-body')).toHaveTextContent('Committed the change.')
    expect(screen.getByTestId('run-status-result-source')).toHaveTextContent(/from Commit Findings$/)
    expect(screen.getByTestId('run-status-item')).toHaveTextContent('Completed · 14m')
    // A success exit's evidence is a run fact, not a line above the result.
    expect(screen.queryByTestId('run-status-outcome-reason')).not.toBeInTheDocument()
    expect(screen.getByTestId('run-fact-outcome')).toHaveTextContent(/commit chose done$/)
    const didNotPass = screen.getByTestId('run-status-did-not-pass')
    expect(within(didNotPass).getByRole('heading')).toHaveTextContent(/^Errors\s*1$/)
    expect(didNotPass).toHaveTextContent('Review: Missing a test. · wrote review.required_changes')
    fireEvent.click(within(didNotPass).getByTestId('run-visit-link'))
    expect(onSelectVisit).toHaveBeenCalledWith(review)

    expect(screen.getByTestId('run-output-commit')).toHaveTextContent('abcdef0 on spark/run-1')
    const files = screen.getAllByTestId('run-output-file')
    expect(files.map((file) => file.textContent)).toEqual(['result.md', 'flow snapshot'])
    fireEvent.click(files[0])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({ path: 'result/result.md' }))
    fireEvent.click(files[1])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({ path: 'artifacts/flow/flow-definition.json' }))
    expect(screen.getByTestId('run-status-facts')).not.toHaveAttribute('open')
  })

  it('keeps a gate note result as written, not as Markdown', () => {
    const note = 'Ship it.\n  - keep the flag off\n\n\nThanks'
    render(
      <RunStatusItem
        run={run({ outcome: 'success' })} kind="completed" now={Date.parse('2026-09-29T11:00:00Z')} visits={[]}
        flowNodes={{ approve: { label: 'Approve', kind: 'human_gate', readsContext: [], command: null } }}
        result={{ ...result, source_node_id: 'approve', body_markdown: note }} resultError={null}
        artifactEntries={[]} question={null} onSelectVisit={vi.fn()} onViewArtifact={vi.fn()}
      />,
    )
    const body = screen.getByTestId('run-result-body')
    expect(body.querySelector('p')?.textContent).toBe(note.trim())
    expect(body.querySelector('p')).toHaveClass('whitespace-pre-wrap')
  })

  it('does not read a completed run that reached a failure exit as a success', () => {
    const blocked = run({ outcome: 'failure', outcome_reason_code: 'blocked', outcome_reason_message: 'review chose blocked: no tests' })
    const props = {
      run: blocked, kind: 'completed' as const, now: Date.parse('2026-09-29T11:00:00Z'), visits: [], flowNodes: {}, resultError: null,
      artifactEntries: [], question: null, onSelectVisit: vi.fn(), onViewArtifact: vi.fn(),
    }
    const { rerender } = render(<RunStatusItem {...props} result={{ ...result, state: 'unavailable', body_markdown: '' }} />)
    const item = screen.getByTestId('run-status-item')
    expect(item).toHaveTextContent('Blocked · 14m')
    expect(item).not.toHaveTextContent('Completed ·')
    // Without a result of its own, the route it took says why.
    expect(screen.getByTestId('run-status-outcome-reason')).toHaveTextContent(/^review chose blocked: no tests$/)
    expect(screen.getByTestId('run-status-outcome-reason')).toHaveClass('text-warning')

    // The flow's own result says why; the route is then a run fact, as for a success exit.
    rerender(<RunStatusItem {...props} result={result} />)
    expect(screen.queryByTestId('run-status-outcome-reason')).not.toBeInTheDocument()
    expect(screen.getByTestId('run-result-body')).toHaveTextContent('Committed the change.')
    expect(screen.getByTestId('run-fact-outcome')).toHaveTextContent(/review chose blocked: no tests$/)
    // Nothing shows above a result still loading.
    rerender(<RunStatusItem {...props} result={null} />)
    expect(screen.queryByTestId('run-status-outcome-reason')).not.toBeInTheDocument()
  })

  it('indents a raw JSON result in a code block', () => {
    render(
      <RunStatusItem
        run={run({ outcome: 'success' })}
        kind="completed"
        now={Date.parse('2026-09-29T11:00:00Z')}
        visits={[]}
        flowNodes={{}}
        result={{ ...result, body_markdown: '{"decision":"done"}' }}
        resultError={null}
        artifactEntries={[]}
        question={null}
        onSelectVisit={vi.fn()}
        onViewArtifact={vi.fn()}
      />,
    )
    expect(screen.getByTestId('run-result-body').querySelector('pre code')).toHaveTextContent('{ "decision": "done" }')
  })

  it('shows a failed run failure, its kind, and the visit it stopped in', () => {
    const stopped = visit('implement', 3, 'failed')
    const error = 'codergen backend failed: codex app-server exited unexpectedly'
    const { onSelectVisit } = renderStatus(
      run({ status: 'failed', last_error: error }),
      [visit('start', 1, 'completed'), visit('implement', 2, 'error', { reason: error, failureKind: 'runtime' }), stopped],
    )

    expect(screen.getByRole('heading', { level: 2 })).toHaveTextContent('Why it failed')
    // Plain words lead; the runtime's own text follows as the detail.
    expect(screen.getByTestId('run-status-failure')).toHaveTextContent(/^The agent could not run\.$/)
    expect(screen.getByTestId('run-status-failure-detail')).toHaveTextContent(error)
    // An earlier visit that hit the same error doesn't repeat it.
    const earlier = screen.getByTestId('run-status-did-not-pass')
    expect(within(earlier).getByRole('heading')).toHaveTextContent(/^Earlier errors\s*1$/)
    expect(earlier).toHaveTextContent('Implement: the same error as above')
    expect(earlier).not.toHaveTextContent('exited unexpectedly')
    expect(screen.getByTestId('run-status-failure-kind')).toHaveAttribute('data-kind', 'infrastructure')
    expect(screen.queryByTestId('run-status-failure-output')).toBeNull()
    fireEvent.click(within(screen.getByTestId('run-status-stopped-in')).getByTestId('run-visit-link'))
    expect(onSelectVisit).toHaveBeenCalledWith(stopped)
    // A failed run's commit is where it started, so it isn't an output.
    expect(screen.queryByTestId('run-output-commit')).not.toBeInTheDocument()
  })

  it("says which branch ended a fan-out in plain words, with its output, and doesn't list it again as an earlier error", () => {
    const reason = 'tool exited with unmapped code 2; stderr: TS2322'
    const branch = (fork: string, number: number, outcome: VisitOutcome) => visit('typecheck', number, outcome, {
      key: `${fork}-typecheck`, parentKey: fork, kind: 'tool', reason, number, count: 2,
      status: { context_updates: { 'tool.output': 'Found 1 error.', 'tool.exit_code': 2 } },
    })
    const forkReason = `branch typecheck (index 0) failed at typecheck: ${reason}`
    // The fork failed, was retried and failed again the same way.
    renderStatus(run({ status: 'failed', last_error: forkReason }), [
      visit('checks', 1, 'error', { kind: 'parallel', reason: forkReason, count: 2 }),
      branch('checks-1', 1, 'error'),
      visit('checks', 2, 'failed', { kind: 'parallel', reason: forkReason, number: 2, count: 2 }),
      branch('checks-2', 2, 'failed'),
    ])
    expect(screen.getByTestId('run-status-failure')).toHaveTextContent(/^The Typecheck branch failed\. The command failed \(exit code 2\)\.$/)
    expect(screen.getByTestId('run-status-failure-detail')).toHaveTextContent(/^TS2322$/)
    expect(screen.getByTestId('run-status-failure-output')).toHaveTextContent('Found 1 error.')
    const rows = within(screen.getByTestId('run-status-did-not-pass')).getAllByTestId('run-status-did-not-pass-row')
    expect(rows.map((row) => row.textContent)).toEqual(['Checks 1: the same error as above', 'Typecheck 1: the same error as above'])
  })

  it("shows the end of a failed command's stdout beside its stderr, where a test runner names what failed", () => {
    const stdout = [...Array.from({ length: 500 }, (_, index) => `test case_${index} ... ok`), '---- contracts::broken stdout ----', "thread 'contracts::broken' panicked", 'test result: FAILED. 500 passed; 1 failed'].join('\n')
    renderStatus(run({ status: 'failed', last_error: 'tool exited with unmapped code 101; stderr: error: test failed' }), [
      visit('validate', 1, 'failed', { kind: 'tool', status: { context_updates: { 'tool.output': stdout, 'tool.exit_code': 101 } } }),
    ])
    // The summary says the command failed; its stderr is the detail.
    expect(screen.getByTestId('run-status-failure-detail')).toHaveTextContent(/^error: test failed$/)
    const output = screen.getByTestId('run-status-failure-output')
    expect(output).toHaveTextContent(/^Output.*contracts::broken.*test result: FAILED\. 500 passed; 1 failed$/)
    expect(output).not.toHaveTextContent('case_0 ')
  })

  it("shows a long failure output's last lines, where its error is, until asked for all", () => {
    const output = [...Array.from({ length: 400 }, (_, index) => `Compiling crate-${index}`), 'error: test failed'].join('\n')
    renderStatus(run({ status: 'failed', last_error: `tool exited with code 101; stderr: ${output}` }), [])
    const detail = screen.getByTestId('run-status-failure-detail')
    expect(detail).toHaveTextContent(/error: test failed$/)
    expect(detail).not.toHaveTextContent('crate-0 ')
    expect(detail.textContent!.split('\n')).toHaveLength(13)
    const toggle = within(detail).getByRole('button', { name: 'Show 389 earlier lines' })
    fireEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(detail).toHaveTextContent('Compiling crate-0')
  })

  it('keeps lock, spec, plan and a separate project path among the run facts', () => {
    renderStatus(run({
      project_path: '/tmp/project',
      working_directory: '/tmp/worktree',
      spec_id: 'spec-7',
      plan_id: 'plan-3',
      execution_lock: { identity: 'id', state: 'queued', key: 'repo:/tmp/project', scope: 'project', conflict_policy: 'queue', queue_position: 2 },
    }), [])
    expect(screen.getByTestId('run-fact-directory')).toHaveTextContent('/tmp/worktree')
    expect(screen.getByTestId('run-fact-project')).toHaveTextContent('/tmp/project')
    expect(screen.getByTestId('run-fact-spec')).toHaveTextContent('spec-7')
    expect(screen.getByTestId('run-fact-plan')).toHaveTextContent('plan-3')
    expect(screen.getByTestId('run-fact-lock')).toHaveTextContent('Queued · repo:/tmp/project · scope project · on conflict queue · queue position 2')
  })

  it('leaves out lock, spec, plan and project facts a run does not have', () => {
    renderStatus(run(), [])
    for (const id of ['project', 'spec', 'plan', 'lock']) {
      expect(screen.queryByTestId(`run-fact-${id}`)).not.toBeInTheDocument()
    }
  })

  it('counts finished visits as the Steps list counts them, branches included', () => {
    renderStatus(run({ status: 'running', ended_at: null }), [
      visit('start', 1, 'completed'),
      visit('fan_out', 2, 'running', { kind: 'parallel' }),
      visit('lint', 2, 'completed', { key: 'lint-branch', parentKey: 'fan_out-2' }),
      visit('unit', 2, 'running', { key: 'unit-branch', parentKey: 'fan_out-2' }),
    ])
    expect(screen.getByTestId('run-status-running')).toHaveTextContent('Now in Unit.')
    expect(screen.getByTestId('run-status-finished-count')).toHaveTextContent(/^2 of 4 visits finished so far\./)
  })

  it('shows the pending question and the visit that asked it', () => {
    renderStatus(run({ status: 'running', ended_at: null }), [visit('review', 1, 'waiting')], true)

    expect(screen.getByTestId('run-status-item')).toHaveAttribute('data-status-kind', 'waiting')
    expect(screen.getByText('Approve the change?')).toBeVisible()
    expect(screen.getByTestId('run-visit-link')).toHaveTextContent('Review')
  })
})

describe('RunContextItem', () => {
  it('shows a key history across visits, including a cleared value, and counts system keys', () => {
    const visits = [
      visit('review', 1, 'error', { status: { context_updates: { 'context.review.findings': 'missing test' } } }),
      visit('evaluate', 2, 'completed', { status: { context_updates: { 'context.review.findings': null, '_attractor.x': 1 } } }),
    ]
    const overview = buildRunContextOverview({ visits, launchContext: { 'context.task.id': 'CR-1' }, finalContext: {} })
    render(
      <RunContextItem
        overview={overview}
        finalContext={{}}
        status="ready"
        contextError={null}
        searchQuery=""
        onSearchQueryChange={vi.fn()}
        contextCopyStatus=""
        contextExportHref={null}
        onCopy={vi.fn()}
        onRefresh={vi.fn()}
        focusKey={null}
        onSelectVisit={vi.fn()}
      />,
    )

    expect(screen.getAllByTestId('run-context-namespace').map((section) => section.dataset.namespace)).toEqual(['review', 'task'])
    const findings = screen.getAllByTestId('run-context-row')[0]
    expect(findings).toHaveTextContent('review.findings')
    expect(findings).toHaveTextContent('cleared by Evaluate')
    fireEvent.click(within(findings).getByTestId('run-context-history-toggle'))
    expect(within(findings).getAllByTestId('run-context-history-entry').map((entry) => entry.textContent)).toEqual([
      'Review:missing test',
      'Evaluate:cleared',
    ])
    expect(screen.getAllByTestId('run-context-row')[1]).toHaveTextContent('launch input')
    expect(screen.getByTestId('run-context-runtime-group')).toHaveTextContent('1 system key')
  })

  it('reads long multiline and structured values in full, including history, from the keyboard', () => {
    const findings = 'Blocking: '.padEnd(500, 'x') + '\nSecond finding line.'
    const visits = [
      visit('review', 1, 'error', { status: { context_updates: { 'context.review.summary': findings } } }),
      visit('review', 2, 'completed', { status: { context_updates: { 'context.review.summary': 'Approved.' } } }),
    ]
    const plan = { steps: ['a', 'b'] }
    const finalContext = { 'context.review.summary': 'Approved.', 'context.task.plan': plan }
    const overview = buildRunContextOverview({ visits, launchContext: { 'context.task.plan': plan }, finalContext })
    render(
      <RunContextItem
        overview={overview}
        finalContext={finalContext}
        status="ready"
        contextError={null}
        searchQuery=""
        onSearchQueryChange={vi.fn()}
        contextCopyStatus=""
        contextExportHref={null}
        onCopy={vi.fn()}
        onRefresh={vi.fn()}
        focusKey={null}
        onSelectVisit={vi.fn()}
      />,
    )
    const [summary, planRow] = screen.getAllByTestId('run-context-row')
    // Compact by default.
    expect(within(planRow).getByTestId('run-context-row-value')).toHaveClass('truncate')
    expect(within(summary).queryByTestId('run-context-history')).not.toBeInTheDocument()

    // The toggle is a native button, so Enter and Space (and taps) reach it.
    const historyToggle = within(summary).getByRole('button', { name: 'written 2×' })
    historyToggle.focus()
    expect(historyToggle).toHaveFocus()
    fireEvent.click(historyToggle)
    const entries = within(summary).getAllByTestId('run-context-history-entry')
    expect(entries[0].textContent).toContain(findings)
    expect(entries[0].lastElementChild).toHaveClass('whitespace-pre-wrap')
    expect(entries[0].lastElementChild).not.toHaveClass('truncate')

    const planToggle = within(planRow).getByRole('button', { name: 'show full value' })
    expect(planToggle).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(planToggle)
    expect(planToggle).toHaveAttribute('aria-expanded', 'true')
    const value = within(planRow).getByTestId('run-context-row-value')
    expect(value.textContent).toBe(JSON.stringify(plan, null, 2))
    expect(value).not.toHaveClass('truncate')
  })

  it('searches system key values and reads them in full while keeping them collapsed', () => {
    const snapshot = { model: 'gpt-6-astra', notes: 'n'.padEnd(200, 'n') }
    const finalContext = {
      'context.task.id': 'CR-1',
      '_attractor.runtime.launch_model': 'gpt-6-astra',
      'internal.execution_configuration_snapshot': snapshot,
    }
    const overview = buildRunContextOverview({ visits: [], launchContext: {}, finalContext })
    const props = {
      overview,
      finalContext,
      status: 'ready' as const,
      contextError: null,
      onSearchQueryChange: vi.fn(),
      contextCopyStatus: '',
      contextExportHref: null,
      onCopy: vi.fn(),
      onRefresh: vi.fn(),
      focusKey: null,
      onSelectVisit: vi.fn(),
    }
    const { rerender } = render(<RunContextItem {...props} searchQuery="" />)
    expect(screen.getByTestId('run-context-runtime-group')).not.toHaveAttribute('open')

    rerender(<RunContextItem {...props} searchQuery="gpt-6-astra" />)
    expect(screen.getByTestId('run-context-runtime-group')).toHaveAttribute('open')
    expect(screen.getAllByTestId('run-context-system-row').map((row) => row.dataset.contextKey)).toEqual([
      '_attractor.runtime.launch_model',
      'internal.execution_configuration_snapshot',
    ])
    expect(screen.queryByTestId('run-context-empty')).not.toBeInTheDocument()

    const row = screen.getAllByTestId('run-context-system-row')[1]
    fireEvent.click(within(row).getByRole('button', { name: 'show full value' }))
    const value = within(row).getByTestId('run-context-system-value')
    expect(value.textContent).toBe(JSON.stringify(snapshot, null, 2))
    expect(value).not.toHaveClass('truncate')
  })
})

// The card follows the page's node selection, as RunsPanel wires it.
function SelectingVisitsCard(props: Omit<ComponentProps<typeof RunVisitsCard>, 'selectedNodeId' | 'onSelectNode'>) {
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  return <RunVisitsCard {...props} selectedNodeId={selectedNodeId} onSelectNode={setSelectedNodeId} />
}

describe('RunVisitsCard', () => {
  it('opens on the status item, opens a written key in the Context item, and links a visit to its files', () => {
    const review = visit('review', 1, 'completed', { status: { context_updates: { 'context.review.findings': 'x' } } })
    const onViewArtifact = vi.fn()
    render(
      <SelectingVisitsCard
        visits={[review]}
        flowNodes={{}}
        segments={[]}
        prompts={[]}
        journal={[]}
        now={0}
        isNarrowViewport={false}
        isLive={false}
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Result' }}
        renderStatus={() => <p>status item</p>}
        renderContext={(focusKey) => (
          <p data-context-key={focusKey ?? undefined}>context item focused on {focusKey}</p>
        )}
        artifactEntries={[
          { path: 'logs/review/executions/1-0/initial-context.txt', size_bytes: 1, media_type: 'text/plain', viewable: true, context_capture_kind: 'codex_turn_input' },
          { path: 'logs/review/executions/1-0/prompt.md', size_bytes: 1, media_type: 'text/markdown', viewable: true },
          { path: 'logs/review/executions/1-0/status.json', size_bytes: 1, media_type: 'application/json', viewable: true },
          { path: 'logs/review/executions/1-0/events.jsonl', size_bytes: 1, media_type: 'application/jsonl', viewable: true },
        ]}
        onViewArtifact={onViewArtifact}
      />,
    )

    expect(screen.getByTestId('run-visit-item-status')).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('status item')).toBeVisible()

    fireEvent.click(screen.getByTestId('run-visit-row'))
    const files = screen.getAllByTestId('run-visit-file')
    expect(files.map((file) => file.textContent)).toEqual(['initial-context', 'prompt', 'status'])
    fireEvent.click(files[2])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({ path: 'logs/review/executions/1-0/status.json' }))
    // The capture keeps its metadata, so the viewer can show its note.
    fireEvent.click(files[0])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({
      path: 'logs/review/executions/1-0/initial-context.txt',
      context_capture_kind: 'codex_turn_input',
    }))

    fireEvent.click(screen.getByTestId('run-visit-context-key'))
    expect(screen.getByTestId('run-visit-item-context')).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('context item focused on context.review.findings')).toBeVisible()
  })

  it('marks a looped-back visit generically from its next node, numbers repeated visits, and switches visits from the control', () => {
    const evaluate1 = visit('evaluate', 2, 'completed', {
      number: 1, count: 2, reason: 'Missing a test.', next: { key: 'implement-3', loopBack: true, retry: false }, decision: 'revise',
      status: { status: 'completed', decision: 'revise', context_updates: { 'context.review.approved': null, 'context.review.notes': 'Add one.' } },
    })
    const implement2 = visit('implement', 3, 'completed', { number: 2, count: 2, next: { key: 'evaluate-4', loopBack: false, retry: false } })
    const evaluate2 = visit('evaluate', 4, 'completed', { number: 2, count: 2, startedAt: '2026-09-29T10:00:00Z', endedAt: '2026-09-29T10:07:43Z' })
    const onToggleGraph = vi.fn()
    render(
      <SelectingVisitsCard
        visits={[evaluate1, implement2, evaluate2]}
        flowNodes={{}}
        segments={[]}
        prompts={[]}
        journal={[]}
        now={0}
        isNarrowViewport={false}
        isLive={false}
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Result', detail: '39m' }}
        contextKeyCount={3}
        renderStatus={() => null}
        renderContext={() => null}
        artifactEntries={[]}
        onViewArtifact={vi.fn()}
        graph={{ shown: false, onToggle: onToggleGraph }}
      />,
    )

    // The rail: a Steps / Journal track with the additive Graph toggle beside it, then the
    // Result and Context rows, then one row per visit.
    expect(screen.getByRole('group', { name: 'View' })).toHaveTextContent('StepsJournal')
    expect(within(screen.getByRole('group', { name: 'View' })).queryByTestId('run-graph-toggle')).toBeNull()
    expect(screen.getByTestId('run-graph-toggle')).toHaveTextContent('Graph')
    expect(screen.getByTestId('run-graph-toggle')).toHaveAttribute('aria-pressed', 'false')
    fireEvent.click(screen.getByTestId('run-graph-toggle'))
    expect(onToggleGraph).toHaveBeenCalledTimes(1)
    expect(screen.getByTestId('run-visit-item-status')).toHaveTextContent('Result39m')
    expect(screen.getByTestId('run-visit-item-context')).toHaveTextContent('Context3 keys')

    const rows = screen.getAllByTestId('run-visit-row')
    expect(rows.map((row) => within(row).getByTestId('run-visit-row-count').textContent)).toEqual(['1', '2', '2'])
    // Only the visit that looped back carries a mark, muted as a loop is ordinary, with a second line.
    expect(rows[0].querySelector('[data-mark]')).toHaveAttribute('data-mark', 'loop_back')
    expect(rows[0].querySelector('[data-mark]')).toHaveClass('text-muted-foreground')
    expect(within(rows[0]).getByTestId('run-visit-row-next-note')).toHaveTextContent('Looped back to Implement')
    expect(rows[1].querySelector('[data-mark]')).toBeNull()
    expect(rows[2].querySelector('[data-mark]')).toBeNull()
    expect(within(rows[2]).getByText('7m 43s')).toBeVisible()

    fireEvent.click(rows[0])
    expect(rows[0]).toHaveAttribute('aria-selected', 'true')
    // Returned: the node status pill, the decision apart from it, where it went, the reason, and the keys it touched.
    const returned = screen.getByTestId('run-visit-returned')
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveTextContent('Completed')
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveAttribute('data-tone', 'success')
    expect(within(returned).getByTestId('run-visit-decision')).toHaveTextContent('Decision revise')
    expect(within(returned).queryByTestId('run-visit-reason-detail')).toBeNull()
    expect(returned).toHaveTextContent('Looped back to')
    expect(screen.getByTestId('run-visit-next')).toHaveTextContent('Implement 2')
    expect(screen.getByTestId('run-visit-reason')).toHaveTextContent('Missing a test.')
    const [cleared, written] = screen.getAllByTestId('run-visit-write')
    expect(cleared).toHaveTextContent(/^review\.approvedcleared$/)
    expect(written).toHaveTextContent(/^review\.notesAdd one\./)
    expect(screen.getByTestId('run-visit-ended')).toHaveTextContent('Visit ended · Completed')

    // The link opens the visit it went to, whose own return is an ordinary "Next".
    fireEvent.click(screen.getByTestId('run-visit-next'))
    expect(rows[1]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByTestId('run-visit-returned')).toHaveTextContent('Next')
    expect(screen.getByTestId('run-visit-next')).toHaveTextContent('Evaluate 2')

    fireEvent.click(rows[0])
    const picker = screen.getByTestId('run-visit-picker')
    expect(within(picker).getByRole('button', { name: 'Visit 1 of 2, looped back' })).toHaveAttribute('aria-pressed', 'true')
    fireEvent.click(within(picker).getByRole('button', { name: 'Visit 2 of 2' }))
    expect(rows[2]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByTestId('run-visit-view')).toHaveTextContent('Visit 2 of 2 · 7m 43s')
    expect(screen.getByTestId('run-visit-view-outcome')).toHaveTextContent('Completed')
    expect(screen.getByTestId('run-visit-ended')).toHaveTextContent('Visit ended · Completed · 7m 43s')
  })

  it('folds a visit transcript into command groups, showing thinking by default and live thinking while it runs', () => {
    const segment = (id: string, order: number, extra: Partial<RunTranscriptSegment>): RunTranscriptSegment => ({
      id, turn_id: 'response', order, kind: 'assistant_message', role: 'assistant', status: 'complete',
      timestamp: '2026-09-29T10:00:00Z', updated_at: '2026-09-29T10:00:00Z', content: '',
      node_id: 'review', stage_index: 1, attempt: 0, latest_sequence: order, source_scope: 'root',
      source_flow_name: null, source_parent_node_id: null, source_run_id: 'run-1',
      ...extra,
    })
    const call = (id: string, order: number, command: string, status: 'completed' | 'failed' = 'completed') => segment(id, order, {
      kind: 'tool_call',
      tool_call: { id, kind: 'command_execution', status, title: 'Ran command', command, output: `${id} output`, file_paths: [] },
    })
    render(
      <SelectingVisitsCard
        visits={[visit('review', 1, 'running')]}
        flowNodes={{}}
        segments={[
          call('a', 1, "/bin/zsh -lc 'git status --short'"),
          segment('think', 2, { kind: 'reasoning', content: '**Checking the diff**\n\nLooks tidy.' }),
          call('b', 3, "/bin/zsh -lc 'npm test'", 'failed'),
          segment('say', 4, { content: 'The tests fail on the loop.' }),
          call('c', 5, 'cat AGENTS.md'),
        ]}
        prompts={[]}
        journal={[]}
        now={0}
        isNarrowViewport={false}
        isLive
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Running', tone: 'info' }}
        contextKeyCount={0}
        renderStatus={() => null}
        renderContext={() => null}
        artifactEntries={[]}
        onViewArtifact={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByTestId('run-visit-row'))
    // A running visit shows its live state beside the title and has not returned yet.
    expect(screen.getByTestId('run-visit-view-outcome')).toHaveTextContent('Running')
    expect(screen.queryByTestId('run-visit-returned')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-visit-ended')).not.toBeInTheDocument()

    const transcript = screen.getByTestId('run-visit-transcript')
    const groups = () => within(transcript).queryAllByRole('button', { expanded: false }).filter((button) => button.dataset.testid?.startsWith('run-tool-group-'))
    // Thinking shows by default, and the calls around it stay plain rows, as does a lone call.
    expect(screen.getByRole('button', { name: 'Hide thinking' })).toBeInTheDocument()
    expect(screen.getByText('Checking the diff')).toBeVisible()
    expect(groups()).toEqual([])
    expect(within(transcript).getByTestId('run-tool-call-toggle-c')).toHaveTextContent('cat AGENTS.md')
    expect(transcript).toHaveTextContent('The tests fail on the loop.')
    expect(within(transcript).getByText('git status --short')).toBeVisible()
    expect(within(transcript).getByText('npm test').closest('button')).toHaveAttribute('title', "/bin/zsh -lc 'npm test'")
    expect(within(transcript).getByText('Failed')).toBeVisible()
    expect(screen.queryByTestId('run-visit-thinking')).toBeNull()
    act(() => useLiveThinkingStore.getState().report(visitKey('run-1', 'review', 1, 0), true, 1200))
    expect(screen.getByTestId('run-visit-thinking')).toHaveTextContent('~1,200 tokens')
    act(() => useLiveThinkingStore.getState().report(visitKey('run-1', 'review', 1, 0), false))
    expect(screen.queryByTestId('run-visit-thinking')).toBeNull()
    // With thinking off, the calls it separated fold into a group.
    fireEvent.click(screen.getByRole('button', { name: 'Hide thinking' }))
    expect(screen.queryByText('Checking the diff')).not.toBeInTheDocument()
    expect(groups().map((group) => group.textContent)).toEqual(['Ran 2 commands· 1 failed'])
    fireEvent.click(screen.getByRole('button', { name: 'Show thinking' }))
    expect(screen.getByText('Checking the diff')).toBeVisible()
    // A line still expands to that call's output.
    fireEvent.click(within(transcript).getByText('npm test'))
    expect(within(transcript).getByText('b output')).toBeVisible()
  })

  it('shows live thinking only while the visit runs', () => {
    useLiveThinkingStore.getState().report(visitKey('run-1', 'review', 1, 0), true, 5)
    render(
      <SelectingVisitsCard
        visits={[visit('review', 1, 'completed')]}
        flowNodes={{}}
        segments={[]}
        prompts={[]}
        journal={[]}
        now={0}
        isNarrowViewport={false}
        isLive={false}
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Completed', tone: 'success' }}
        contextKeyCount={0}
        renderStatus={() => null}
        renderContext={() => null}
        artifactEntries={[]}
        onViewArtifact={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByTestId('run-visit-row'))
    expect(screen.getByTestId('run-visit-view-outcome')).toBeVisible()
    expect(screen.queryByTestId('run-visit-thinking')).toBeNull()
    act(() => useLiveThinkingStore.getState().report(visitKey('run-1', 'review', 1, 0), false))
  })

  it('clears a search that hides a key opened from a visit', () => {
    const review = visit('review', 1, 'completed', { status: { context_updates: { 'context.review.findings': 'x' } } })
    const overview = buildRunContextOverview({
      visits: [review],
      launchContext: { 'context.request.objective': 'Finish the Runs page' },
      finalContext: { 'context.review.findings': 'x', 'context.request.objective': 'Finish the Runs page' },
    })
    function Harness() {
      const [query, setQuery] = useState('')
      return (
        <SelectingVisitsCard
          visits={[review]}
          flowNodes={{}}
          segments={[]}
          prompts={[]}
          journal={[]}
          now={0}
          isNarrowViewport={false}
          isLive={false}
          timelineError={null}
          onOpenRun={vi.fn()}
          statusRow={{ label: 'Result' }}
          renderStatus={() => null}
          renderContext={(focusKey, selectVisit) => (
            <RunContextItem
              overview={overview}
              finalContext={null}
              status="ready"
              contextError={null}
              searchQuery={query}
              onSearchQueryChange={setQuery}
              contextCopyStatus=""
              contextExportHref={null}
              onCopy={vi.fn()}
              onRefresh={vi.fn()}
              focusKey={focusKey}
              onSelectVisit={selectVisit}
            />
          )}
          artifactEntries={[]}
          onViewArtifact={vi.fn()}
        />
      )
    }
    render(<Harness />)
    fireEvent.click(screen.getByTestId('run-visit-item-context'))
    fireEvent.change(screen.getByTestId('run-context-search-input'), { target: { value: 'objective' } })
    expect(screen.getAllByTestId('run-context-row').map((row) => row.dataset.contextKey)).toEqual(['context.request.objective'])

    fireEvent.click(screen.getByTestId('run-visit-row'))
    fireEvent.click(screen.getByTestId('run-visit-context-key'))
    expect(screen.getByTestId('run-context-search-input')).toHaveValue('')
    expect(screen.getAllByTestId('run-context-row').map((row) => row.dataset.contextKey)).toContain('context.review.findings')

    // Opening the Context item itself keeps the reader's own search.
    fireEvent.change(screen.getByTestId('run-context-search-input'), { target: { value: 'objective' } })
    fireEvent.click(screen.getByTestId('run-visit-row'))
    fireEvent.click(screen.getByTestId('run-visit-item-context'))
    expect(screen.getByTestId('run-context-search-input')).toHaveValue('objective')
  })
})

describe('RunVisitsCard returned evidence', () => {
  const openOnly = (visits: RunVisit[], flowNodes: Record<string, VisitFlowNode> = {}) => {
    render(
      <SelectingVisitsCard
        visits={visits}
        flowNodes={flowNodes}
        segments={[]}
        prompts={[]}
        journal={[]}
        now={0}
        isNarrowViewport={false}
        isLive={false}
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Result' }}
        contextKeyCount={0}
        renderStatus={() => null}
        renderContext={() => null}
        artifactEntries={[]}
        onViewArtifact={vi.fn()}
      />,
    )
    fireEvent.click(screen.getAllByTestId('run-visit-row')[0])
    return screen.getByTestId('run-visit-returned')
  }

  it('tells a retried visit from a loop: a muted note, no loop mark, and "Retried as" its next attempt', () => {
    const returned = openOnly([
      visit('check', 1, 'error', { number: 1, count: 2, failureKind: 'runtime', reason: 'tsc exited 2', next: { key: 'check-2', loopBack: false, retry: true } }),
      visit('check', 2, 'completed', { key: 'check-2', number: 2, count: 2 }),
    ])
    const [first] = screen.getAllByTestId('run-visit-row')
    expect(first.querySelector('[data-mark]')).toHaveAttribute('data-mark', 'error')
    expect(within(first).getByTestId('run-visit-row-next-note')).toHaveTextContent(/^Retried$/)
    expect(returned).toHaveTextContent('Retried as')
    expect(returned).not.toHaveTextContent('Looped back')
    expect(within(returned).getByTestId('run-visit-next')).toHaveTextContent('Check 2')
    expect(screen.getByRole('button', { name: 'Visit 1 of 2, retried' })).toBeInTheDocument()
  })

  it('shows an error status with its kind and no decision', () => {
    const returned = openOnly([visit('validate', 1, 'error', {
      failureKind: 'contract', reason: 'response named no declared decision',
      status: { status: 'error', decision: null, failure_kind: 'contract', tolerable: false },
    })])
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveTextContent('Error')
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveAttribute('data-tone', 'warning')
    // The kind sits beside the status in plain words, not as a line of runtime text of its own.
    expect(within(returned).getByTestId('run-visit-error-kind')).toHaveTextContent(/^contract error$/)
    expect(within(returned).queryByTestId('run-visit-reason-detail')).toBeNull()
    expect(within(returned).queryByTestId('run-visit-decision')).toBeNull()
    expect(within(returned).getByTestId('run-visit-reason')).toHaveTextContent('response named no declared decision')
  })

  it("leads a runtime failure with plain words and keeps the kind and the command's stderr as detail", () => {
    const reason = 'tool exited with unmapped code 1; stderr: lint broke'
    const returned = openOnly([visit('lint', 1, 'failed', {
      kind: 'tool', failureKind: 'runtime', reason,
      status: { status: 'error', failure_kind: 'runtime', tolerable: true, failure_reason: reason },
    })])
    expect(within(returned).getByTestId('run-visit-reason')).toHaveTextContent(/^The command failed \(exit code 1\)\.$/)
    expect(within(returned).getByTestId('run-visit-error-kind')).toHaveTextContent(/^runtime error$/)
    expect(within(returned).getByTestId('run-visit-reason-detail')).toHaveTextContent(/^lint broke$/)
  })

  it("shows a tool's stdout once, under its command", () => {
    const returned = openOnly([visit('check', 1, 'completed', {
      kind: 'tool', decision: 'done', reason: '42 passed',
      status: { status: 'completed', decision: 'done', notes: '42 passed', context_updates: { 'tool.output': '42 passed', 'tool.exit_code': 0 } },
    })])
    expect(within(returned).queryByTestId('run-visit-reason')).toBeNull()
    expect(screen.getByTestId('run-visit-tool')).toHaveTextContent('42 passed')
    expect(screen.getAllByText('42 passed')).toHaveLength(1)
  })

  it("opens a failed command's output at its end, where the error is, and reaches it from the keyboard", () => {
    vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockReturnValue(900)
    const tool = (outcome: VisitOutcome, exitCode: number) => visit('check', 1, outcome, {
      kind: 'tool', status: { context_updates: { 'tool.output': 'line 1\nline 2', 'tool.exit_code': exitCode } },
    })
    openOnly([tool('failed', 101)])
    const output = screen.getByRole('region', { name: 'Command output' })
    expect(output).toHaveAttribute('tabindex', '0')
    expect(output.scrollTop).toBe(900)
    cleanup()

    openOnly([tool('completed', 0)])
    expect(screen.getByTestId('run-visit-tool-output').scrollTop).toBe(0)
    vi.restoreAllMocks()
  })

  it("shows a failure's output once in the journal, under the step, when the run fails with the same error", () => {
    const error = `tool exited with unmapped code 101; stderr: ${Array.from({ length: 20 }, (_, index) => `line ${index}`).join('\n')}`
    const entry = (sequence: number, type: string, payload: Record<string, unknown>): TimelineEventEntry => ({
      id: `event-${sequence}`, sequence, type, category: 'lifecycle', severity: 'error', nodeId: 'validate', stageIndex: 1,
      summary: type, receivedAt: '2026-09-29T10:00:00Z', sourceScope: 'root', sourceParentNodeId: null, sourceFlowName: null, payload,
    })
    render(
      <SelectingVisitsCard
        visits={[]}
        flowNodes={{ validate: { label: 'Validate', kind: 'tool', readsContext: [], command: null } }}
        segments={[]}
        prompts={[]}
        journal={[entry(1, 'StageFailed', { error }), entry(2, 'PipelineFailed', { error })]}
        now={0}
        isNarrowViewport={false}
        isLive={false}
        timelineError={null}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Failed' }}
        contextKeyCount={0}
        renderStatus={() => null}
        renderContext={() => null}
        artifactEntries={[]}
        onViewArtifact={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByTestId('run-journal-toggle'))
    const [stepFailed, runFailed] = screen.getAllByTestId('run-journal-row')
    expect(stepFailed).toHaveTextContent('Validate failed: The command failed (exit code 101).')
    expect(stepFailed).toHaveTextContent('line 19')
    expect(runFailed).toHaveTextContent(/Run failed: The command failed \(exit code 101\)\.$/)
    expect(runFailed).not.toHaveTextContent('line 19')
  })

  it('shows a gate only its result, the answer by its label: no empty transcript or instructions', () => {
    openOnly(
      [visit('approve', 1, 'completed', { kind: 'human_gate', decision: 'approve', status: { status: 'completed', decision: 'approve' } })],
      { approve: { label: 'Approve', kind: 'human_gate', readsContext: [], command: null, routeLabels: { approve: 'Approve the release' } } },
    )
    expect(screen.getByTestId('run-visit-view-outcome')).toHaveTextContent('Completed')
    expect(screen.getByTestId('run-visit-decision')).toHaveTextContent(/^Answer Approve the release$/)
    expect(screen.queryByTestId('run-visit-work')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-visit-instructions')).not.toBeInTheDocument()
    expect(screen.queryByText('No transcript was recorded for this visit.')).not.toBeInTheDocument()
  })

  it('lists each fan-out branch by its step name with failed, terminal-result and interrupted evidence', () => {
    const results = [
      { id: 'lint', index: 0, status: 'completed', started: true, context: {}, route_trace: [], decisions: [], notes: '' },
      { id: 'typecheck', index: 1, status: 'error', started: true, context: {}, route_trace: [], decisions: [], notes: '', error: { node: 'tsc', kind: 'runtime', tolerable: false, reason: 'tsc exited 2', attempts: 2 } },
      { id: 'review', index: 2, status: 'terminal_result', started: true, context: {}, route_trace: [], decisions: [], notes: '', terminal: { exit_id: 'stop', result: { name: 'blocked', outcome: 'failure' }, branch: 'review', branch_index: 2, node: 'judge', decision: 'block', notes: '' } },
      { id: 'docs', index: 3, status: 'canceled', started: false, context: {}, route_trace: [], decisions: [], notes: '' },
    ]
    const returned = openOnly([visit('fan_out', 1, 'error', {
      kind: 'parallel', failureKind: 'runtime',
      status: { status: 'error', failure_kind: 'runtime', tolerable: false, context_updates: { 'parallel.results': results } },
    }), visit('lint', 1, 'completed', { key: 'lint-branch', label: 'Run the linter', parentKey: 'fan_out-1' })])
    const branches = within(returned).getAllByTestId('run-visit-branch')
    expect(branches.map((branch) => branch.getAttribute('data-status'))).toEqual(['completed', 'error', 'terminal_result', 'canceled'])
    // A branch reads as its step's name, which opens that step; one never started is named from its ID.
    expect(branches[0]).toHaveTextContent(/^Run the linterCompleted$/)
    // Its error's kind reads as a step's does, in the evidence, with no bare runtime line under it,
    // and where past its first step by that step's name.
    expect(within(branches[1]).getByTestId('run-visit-branch-evidence')).toHaveTextContent(/^runtime error at Tsc · 2 attempts$/)
    expect(branches[1]).toHaveTextContent(/^TypecheckError/)
    expect(within(branches[1]).getByTestId('run-visit-branch-reason')).toHaveTextContent('tsc exited 2')
    expect(within(branches[1]).queryByTestId('run-visit-branch-reason-detail')).toBeNull()
    expect(branches[2]).toHaveTextContent('Reached an exit')
    expect(within(branches[2]).getByTestId('run-visit-branch-evidence')).toHaveTextContent('result blocked · failure · from judge')
    // A canceled branch reads as its step does in Steps.
    expect(branches[3]).toHaveTextContent(/^DocsInterrupted$/)
    // parallel.results is runtime bookkeeping, so it is not among the writes.
    expect(within(returned).queryByTestId('run-visit-writes')).toBeNull()
    fireEvent.click(within(branches[0]).getByTestId('run-visit-branch-open'))
    expect(screen.getByTestId('run-visit-view')).toHaveAttribute('data-visit-key', 'lint-branch')
    // Focus follows to the opened step instead of falling to the page.
    expect(screen.getByTestId('run-visit-view-title')).toHaveFocus()
  })

  it('lists the branches of a fan-out a cancel stopped before it recorded results', () => {
    const returned = openOnly([
      visit('fan_out', 1, 'interrupted', { kind: 'parallel' }),
      visit('lint', 1, 'completed', { key: 'lint-branch', label: 'Lint', parentKey: 'fan_out-1' }),
      visit('unit', 1, 'interrupted', { key: 'unit-branch', label: 'Unit tests', parentKey: 'fan_out-1' }),
    ])
    const branches = within(returned).getAllByTestId('run-visit-branch')
    expect(branches.map((branch) => branch.textContent)).toEqual(['LintCompleted', 'Unit testsInterrupted'])
  })
})

describe('RunVisitsCard steering', () => {
  afterEach(() => vi.unstubAllGlobals())

  const visitsCard = (visits: RunVisit[]) => (
    <SelectingVisitsCard
      visits={visits}
      flowNodes={{}}
      segments={[]}
      prompts={[]}
      journal={[]}
      now={0}
      isNarrowViewport={false}
      isLive={true}
      timelineError={null}
      onOpenRun={vi.fn()}
      statusRow={{ label: 'Running' }}
      contextKeyCount={0}
      renderStatus={() => null}
      renderContext={() => null}
      artifactEntries={[]}
      onViewArtifact={vi.fn()}
    />
  )
  const renderVisits = (visits: RunVisit[]) => render(visitsCard(visits))
  const openVisit = (index: number) => fireEvent.click(screen.getAllByTestId('run-visit-row')[index])
  const steerResponse = (body: Record<string, unknown>) => vi.fn(async () => new Response(JSON.stringify(body), { status: 200 }))

  it('offers a message input only on a running agent node', () => {
    renderVisits([
      visit('implement', 1, 'running'),
      visit('check', 2, 'running', { kind: 'tool' }),
      visit('plan', 0, 'completed'),
    ])
    openVisit(0)
    expect(screen.getByTestId('run-visit-steer-input')).toBeVisible()
    openVisit(1)
    expect(screen.queryByTestId('run-visit-steer-input')).toBeNull()
    openVisit(2)
    expect(screen.queryByTestId('run-visit-steer-input')).toBeNull()
  })

  it('posts the message to the node and shows the delivery', async () => {
    const fetchMock = steerResponse({
      pipeline_id: 'run-1', target_run_id: 'run-1', run_id: 'run-1', status: 'delivered', delivery_mode: 'active_turn',
      reason: null, message: 'Sent to the running Claude Code turn.', target_node_id: 'implement',
    })
    vi.stubGlobal('fetch', fetchMock)
    renderVisits([visit('implement', 1, 'running')])
    openVisit(0)
    fireEvent.change(screen.getByTestId('run-visit-steer-input'), { target: { value: 'Use the existing helper.' } })
    fireEvent.click(screen.getByTestId('run-visit-steer-send'))

    expect(await screen.findByTestId('run-visit-steer-delivered')).toHaveTextContent('Sent to the running Claude Code turn.')
    expect(fetchMock).toHaveBeenCalledWith('/attractor/pipelines/run-1/steer', expect.objectContaining({ method: 'POST' }))
    const init = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1]
    expect(JSON.parse(init.body as string)).toEqual({
      message: 'Use the existing helper.', target_run_id: 'run-1', target_node_id: 'implement',
    })
    expect(screen.getByTestId('run-visit-steer-input')).toHaveValue('')
    expect(screen.queryByTestId('run-visit-steer-rejected')).toBeNull()
  })

  it('shows a rejection with its reason and keeps the draft', async () => {
    const message = 'Several agent nodes are running (run-a/node-x, run-b/node-y); choose one to steer.'
    vi.stubGlobal('fetch', steerResponse({
      pipeline_id: 'run-1', target_run_id: 'run-1', run_id: 'run-1', status: 'rejected', delivery_mode: null,
      reason: 'ambiguous_target', message, target_node_id: null,
    }))
    renderVisits([visit('implement', 1, 'running')])
    openVisit(0)
    fireEvent.change(screen.getByTestId('run-visit-steer-input'), { target: { value: 'Stop.' } })
    fireEvent.click(screen.getByTestId('run-visit-steer-send'))

    const rejected = await screen.findByTestId('run-visit-steer-rejected')
    expect(rejected).toHaveTextContent(`${message} (ambiguous_target)`)
    expect(rejected).toHaveAttribute('role', 'alert')
    expect(screen.getByTestId('run-visit-steer-input')).toHaveValue('Stop.')
  })

  it('shows an HTTP error', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ detail: 'Unknown pipeline' }), { status: 404 })))
    renderVisits([visit('implement', 1, 'running')])
    openVisit(0)
    fireEvent.change(screen.getByTestId('run-visit-steer-input'), { target: { value: 'Stop.' } })
    fireEvent.click(screen.getByTestId('run-visit-steer-send'))

    await waitFor(() => expect(screen.getByTestId('run-visit-steer-rejected')).toHaveTextContent('Unable to send message (HTTP 404): Unknown pipeline.'))
  })

  it.each([
    ['before', 'delivered', 'run-visit-steer-delivered'],
    ['before', 'rejected', 'run-visit-steer-rejected'],
    ['after', 'delivered', 'run-visit-steer-delivered'],
    ['after', 'rejected', 'run-visit-steer-rejected'],
  ])('keeps the result when the visit ends %s a %s response', async (when, status, testId) => {
    let respond = (_: Response) => {}
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => { respond = resolve })))
    const view = renderVisits([visit('implement', 1, 'running')])
    openVisit(0)
    fireEvent.change(screen.getByTestId('run-visit-steer-input'), { target: { value: 'Stop.' } })
    fireEvent.click(screen.getByTestId('run-visit-steer-send'))
    const end = () => view.rerender(visitsCard([visit('implement', 1, 'completed')]))
    const reply = () => respond(new Response(JSON.stringify({
      pipeline_id: 'run-1', target_run_id: 'run-1', run_id: 'run-1', status, delivery_mode: null,
      reason: status === 'rejected' ? 'no_active_agent_node' : null, message: `Steer ${status}.`, target_node_id: 'implement',
    }), { status: 200 }))
    if (when === 'before') {
      end()
      reply()
    } else {
      reply()
      await screen.findByTestId(testId)
      end()
    }

    expect(await screen.findByTestId(testId)).toHaveTextContent(`Steer ${status}.`)
    expect(screen.queryByTestId('run-visit-steer-input')).toBeNull()
    expect(screen.queryByTestId('run-visit-steer-send')).toBeNull()
  })
})
