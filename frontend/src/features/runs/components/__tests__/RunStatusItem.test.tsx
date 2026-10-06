import { fireEvent, render, screen, within } from '@testing-library/react'
import { useState, type ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type { PipelineResultResponse } from '@/lib/attractorClient'
import type { RunRecord } from '../../model/shared'
import { buildRunContextOverview, runStatusKind } from '../../model/runOverviewModel'
import type { RunVisit, VisitOutcome } from '../../model/visitModel'
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
        { path: 'artifacts/flow/flow-source.yaml', size_bytes: 10, media_type: 'text/yaml', viewable: true },
      ]}
      question={<p>Approve the change?</p>}
      onSelectVisit={onSelectVisit}
      onViewArtifact={onViewArtifact}
    />,
  )
  return { onSelectVisit, onViewArtifact }
}

describe('RunStatusItem', () => {
  it('shows a completed run result, its source, what did not pass, and its outputs', () => {
    const review = visit('review', 1, 'did_not_pass', {
      reason: 'Missing a test.',
      status: { outcome: 'fail', context_updates: { 'context.review.required_changes': 'Add a test.', 'internal.retry': 1 } },
    })
    const { onSelectVisit, onViewArtifact } = renderStatus(run(), [review, visit('commit', 2, 'succeeded')])

    expect(screen.getByTestId('run-result-body')).toHaveTextContent('Committed the change.')
    expect(screen.getByTestId('run-status-result-source')).toHaveTextContent('from Commit Findings, raw output')
    const didNotPass = screen.getByTestId('run-status-did-not-pass')
    expect(within(didNotPass).getByRole('heading')).toHaveTextContent(/^Didn’t pass\s*1$/)
    expect(didNotPass).toHaveTextContent('Review: Missing a test. · wrote review.required_changes')
    fireEvent.click(within(didNotPass).getByTestId('run-visit-link'))
    expect(onSelectVisit).toHaveBeenCalledWith(review)

    expect(screen.getByTestId('run-output-commit')).toHaveTextContent('abcdef0 on spark/run-1')
    const files = screen.getAllByTestId('run-output-file')
    expect(files.map((file) => file.textContent)).toEqual(['result.md', 'flow snapshot'])
    fireEvent.click(files[0])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({ path: 'result/result.md' }))
    expect(screen.getByTestId('run-status-facts')).not.toHaveAttribute('open')
  })

  it('shows a failed run failure, its kind, and the visit it stopped in', () => {
    const stopped = visit('implement', 2, 'failed')
    const { onSelectVisit } = renderStatus(
      run({ status: 'failed', last_error: 'codergen backend failed: codex app-server exited unexpectedly' }),
      [visit('start', 1, 'succeeded'), stopped],
    )

    expect(screen.getByTestId('run-status-failure')).toHaveTextContent('codex app-server exited unexpectedly')
    expect(screen.getByTestId('run-status-failure-kind')).toHaveAttribute('data-kind', 'infrastructure')
    fireEvent.click(within(screen.getByTestId('run-status-stopped-in')).getByTestId('run-visit-link'))
    expect(onSelectVisit).toHaveBeenCalledWith(stopped)
    // A failed run's commit is where it started, so it isn't an output.
    expect(screen.queryByTestId('run-output-commit')).not.toBeInTheDocument()
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
      visit('review', 1, 'did_not_pass', { status: { context_updates: { 'context.review.findings': 'missing test' } } }),
      visit('evaluate', 2, 'succeeded', { status: { context_updates: { 'context.review.findings': null, '_attractor.x': 1 } } }),
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
      visit('review', 1, 'did_not_pass', { status: { context_updates: { 'context.review.summary': findings } } }),
      visit('review', 2, 'succeeded', { status: { context_updates: { 'context.review.summary': 'Approved.' } } }),
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
    const review = visit('review', 1, 'succeeded', { status: { context_updates: { 'context.review.findings': 'x' } } })
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
        transcriptError={null}
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

  it('marks a sent-back visit generically from its next node, numbers repeated visits, and switches visits from the control', () => {
    const evaluate1 = visit('evaluate', 2, 'did_not_pass', {
      number: 1, count: 2, reason: 'Missing a test.', next: { key: 'implement-3', loopBack: true },
      status: { outcome: 'fail', context_updates: { 'context.review.approved': null, 'context.review.notes': 'Add one.' } },
    })
    const implement2 = visit('implement', 3, 'succeeded', { number: 2, count: 2, next: { key: 'evaluate-4', loopBack: false } })
    const evaluate2 = visit('evaluate', 4, 'succeeded', { number: 2, count: 2, startedAt: '2026-09-29T10:00:00Z', endedAt: '2026-09-29T10:07:43Z' })
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
        transcriptError={null}
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
    // Only the visit that was sent back carries a mark, in the warning tone, with a second line.
    expect(rows[0].querySelector('[data-mark]')).toHaveAttribute('data-mark', 'did_not_pass')
    expect(rows[0].querySelector('[data-mark]')).toHaveClass('text-warning')
    expect(within(rows[0]).getByTestId('run-visit-row-sent-back')).toHaveTextContent('Sent back to Implement')
    expect(rows[1].querySelector('[data-mark]')).toBeNull()
    expect(rows[2].querySelector('[data-mark]')).toBeNull()
    expect(within(rows[2]).getByText('7m 43s')).toBeVisible()

    fireEvent.click(rows[0])
    expect(rows[0]).toHaveAttribute('aria-selected', 'true')
    // Returned: the outcome pill, where it went, the reason, and the keys it touched.
    const returned = screen.getByTestId('run-visit-returned')
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveTextContent("Didn't pass")
    expect(within(returned).getByTestId('run-visit-view-outcome')).toHaveAttribute('data-tone', 'warning')
    expect(returned).toHaveTextContent('Sent back to')
    expect(screen.getByTestId('run-visit-next')).toHaveTextContent('Implement 2')
    expect(screen.getByTestId('run-visit-reason')).toHaveTextContent('Missing a test.')
    const [cleared, written] = screen.getAllByTestId('run-visit-write')
    expect(cleared).toHaveTextContent(/^review\.approvedcleared$/)
    expect(written).toHaveTextContent(/^review\.notesAdd one\./)
    expect(screen.getByTestId('run-visit-ended')).toHaveTextContent("Visit ended · Didn't pass")

    // The link opens the visit it went to, whose own return is an ordinary "Next".
    fireEvent.click(screen.getByTestId('run-visit-next'))
    expect(rows[1]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByTestId('run-visit-returned')).toHaveTextContent('Next')
    expect(screen.getByTestId('run-visit-next')).toHaveTextContent('Evaluate 2')

    fireEvent.click(rows[0])
    const picker = screen.getByTestId('run-visit-picker')
    expect(within(picker).getByRole('button', { name: 'Visit 1 of 2, sent back' })).toHaveAttribute('aria-pressed', 'true')
    fireEvent.click(within(picker).getByRole('button', { name: 'Visit 2 of 2' }))
    expect(rows[2]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByTestId('run-visit-view')).toHaveTextContent('Visit 2 of 2 · 7m 43s')
    expect(screen.getByTestId('run-visit-view-outcome')).toHaveTextContent('Succeeded')
    expect(screen.getByTestId('run-visit-ended')).toHaveTextContent('Visit ended · Succeeded · 7m 43s')
  })

  it('folds a visit transcript into command groups and hides thinking until asked', () => {
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
        transcriptError={null}
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
    const groups = within(transcript).getAllByRole('button', { expanded: false }).filter((button) => button.dataset.testid?.startsWith('run-tool-group-'))
    expect(groups.map((group) => group.textContent)).toEqual(['Ran 2 commands· 1 failed', 'Ran 1 command'])
    expect(transcript).toHaveTextContent('The tests fail on the loop.')
    // Thinking stays hidden until asked, then appears inside the group it sat in.
    expect(screen.queryByText('Checking the diff')).not.toBeInTheDocument()
    fireEvent.click(groups[0])
    expect(within(transcript).getByText('git status --short')).toBeVisible()
    expect(within(transcript).getByText('npm test').closest('button')).toHaveAttribute('title', "/bin/zsh -lc 'npm test'")
    expect(within(transcript).getByText('Failed')).toBeVisible()
    expect(screen.queryByText('Checking the diff')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show thinking' }))
    expect(screen.getByText('Checking the diff')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Show thinking' })).toHaveAttribute('aria-pressed', 'true')
    // A line still expands to that call's output.
    fireEvent.click(within(transcript).getByText('npm test'))
    expect(within(transcript).getByText('b output')).toBeVisible()
  })

  it('clears a search that hides a key opened from a visit', () => {
    const review = visit('review', 1, 'succeeded', { status: { context_updates: { 'context.review.findings': 'x' } } })
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
          transcriptError={null}
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
