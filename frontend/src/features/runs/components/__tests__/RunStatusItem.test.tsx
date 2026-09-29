import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

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
    const review = visit('review', 1, 'did_not_pass', { reason: 'Missing a test.' })
    const { onSelectVisit, onViewArtifact } = renderStatus(run(), [review, visit('commit', 2, 'succeeded')])

    expect(screen.getByTestId('run-result-body')).toHaveTextContent('Committed the change.')
    expect(screen.getByTestId('run-status-result-source')).toHaveTextContent('from Commit Findings, raw output')
    const didNotPass = screen.getByTestId('run-status-did-not-pass')
    expect(didNotPass).toHaveTextContent('Didn’t pass (1)')
    expect(didNotPass).toHaveTextContent('Review: Missing a test.')
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
})

describe('RunVisitsCard', () => {
  it('opens on the status item, opens a written key in the Context item, and links a visit to its files', () => {
    const review = visit('review', 1, 'succeeded', { status: { context_updates: { 'context.review.findings': 'x' } } })
    const onViewArtifact = vi.fn()
    render(
      <RunVisitsCard
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
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        onOpenRun={vi.fn()}
        statusRow={{ label: 'Result' }}
        renderStatus={() => <p>status item</p>}
        renderContext={(focusKey) => (
          <p data-context-key={focusKey ?? undefined}>context item focused on {focusKey}</p>
        )}
        artifactEntries={[
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
    expect(files.map((file) => file.textContent)).toEqual(['prompt', 'status'])
    fireEvent.click(files[1])
    expect(onViewArtifact).toHaveBeenCalledWith(expect.objectContaining({ path: 'logs/review/executions/1-0/status.json' }))

    fireEvent.click(screen.getByTestId('run-visit-context-key'))
    expect(screen.getByTestId('run-visit-item-context')).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('context item focused on context.review.findings')).toBeVisible()
  })
})
