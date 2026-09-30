import { describe, expect, it } from 'vitest'

import type { NodeExecutionResponse, RunTranscriptSegment } from '@/lib/api/attractorApi'
import type { TimelineEventEntry } from '../shared'
import {
  buildRunVisits,
  visitContextWrites,
  visitFlowNodesFromSnapshot,
  visitMark,
  visitPrompt,
  visitTranscriptRows,
  type RunVisit,
  type VisitModelInput,
} from '../visitModel'

const RUN = 'run-loop'

// Journal entries as the journal API maps them: the raw event is the payload.
let clock = 0
const event = (type: string, payload: Record<string, unknown>, sourceScope: 'root' | 'child' = 'root'): TimelineEventEntry => {
  clock += 1
  return {
    id: `event-${clock}`,
    sequence: clock,
    type,
    category: 'stage',
    severity: 'info',
    nodeId: typeof payload.node_id === 'string' ? payload.node_id : null,
    stageIndex: type.startsWith('Stage') && typeof payload.index === 'number' ? payload.index : null,
    summary: type,
    receivedAt: new Date(Date.UTC(2026, 8, 27, 17, 0, clock)).toISOString(),
    sourceScope,
    sourceParentNodeId: null,
    sourceFlowName: null,
    payload,
  }
}

const started = (node: string, index: number) => event('StageStarted', { node_id: node, index })
const completed = (node: string, index: number, extra: Record<string, unknown> = {}) => (
  event('StageCompleted', { node_id: node, index, outcome: 'success', ...extra })
)
const failed = (node: string, index: number, error = 'stage_failed') => (
  event('StageFailed', { node_id: node, index, error, will_retry: false })
)
const stage = (node: string, index: number, end: 'ok' | 'fail' = 'ok') => (
  [started(node, index), end === 'ok' ? completed(node, index) : failed(node, index)]
)

const execution = (node: string, index: number, status: Record<string, unknown> | null): NodeExecutionResponse => ({
  run_id: RUN, node_id: node, stage_index: index, attempt: 0, status,
})

const flowNodes = visitFlowNodesFromSnapshot({
  nodes: {
    start: { kind: 'start', label: 'Start' },
    implement: { kind: 'subflow', label: 'Implement', config: { kind: 'subflow' } },
    evaluate: {
      kind: 'agent_task',
      label: 'Evaluate',
      contracts: { reads_context: ['context.workspace.path', 'context.task.objective'] },
    },
    validate: { kind: 'tool', label: 'Validate', config: { kind: 'tool', command: 'just test' } },
    diagnose: { kind: 'agent_task', label: 'Diagnose' },
    fan_out: { kind: 'parallel', label: 'Fan out' },
    lint: { kind: 'tool', label: 'Lint' },
    typecheck: { kind: 'tool', label: 'Typecheck' },
    ask: { kind: 'human', label: 'Ask' },
    done: { kind: 'exit', label: 'Done' },
  },
})

const build = (overrides: Partial<VisitModelInput>): RunVisit[] => buildRunVisits({
  runId: RUN,
  runStatus: 'completed',
  journal: [],
  executions: [],
  childRuns: [],
  flowNodes,
  waitingNodeIds: [],
  ...overrides,
})

const summarize = (visits: RunVisit[]) => visits.map((visit) => (
  `${visit.label} ${visit.number}/${visit.count} ${visit.outcome}${visit.next?.loopBack ? ' ↩' : ''}`
))

describe('buildRunVisits', () => {
  it('follows a review loop where Evaluate ran four times and two rejections routed back to Implement', () => {
    // Modelled on run-18d93c056079b9c0 (implement-change).
    const journal = [
      ...stage('start', 0),
      ...stage('implement', 1), ...stage('evaluate', 2, 'fail'),
      ...stage('implement', 3), ...stage('evaluate', 4),
      ...stage('validate', 5, 'fail'), ...stage('diagnose', 6),
      ...stage('implement', 7), ...stage('evaluate', 8, 'fail'),
      ...stage('implement', 9), ...stage('evaluate', 10),
      ...stage('validate', 11), ...stage('done', 12),
    ]
    const rejection = {
      outcome: 'fail',
      failure_reason: '',
      notes: '',
      context_updates: {
        'context.review.summary': 'Blocking contract gap.',
        'context.review.required_changes': 'Preserve effort strings.',
        last_stage: 'evaluate',
        last_response: '{}',
      },
    }
    const visits = build({ journal, executions: [execution('evaluate', 2, rejection)] })

    expect(summarize(visits)).toEqual([
      'Start 1/1 succeeded',
      'Implement 1/4 succeeded',
      'Evaluate 1/4 did_not_pass ↩',
      'Implement 2/4 succeeded',
      'Evaluate 2/4 succeeded',
      'Validate 1/2 did_not_pass',
      'Diagnose 1/1 succeeded ↩',
      'Implement 3/4 succeeded',
      'Evaluate 3/4 did_not_pass ↩',
      'Implement 4/4 succeeded',
      'Evaluate 4/4 succeeded',
      'Validate 2/2 succeeded',
      'Done 1/1 succeeded',
    ])
    const firstEvaluate = visits[2]
    expect(visitMark(firstEvaluate)).toBe('did_not_pass')
    expect(firstEvaluate.next).toEqual({ key: visits[3].key, loopBack: true })
    expect(firstEvaluate.startedAt).not.toBeNull()
    expect(firstEvaluate.endedAt).not.toBeNull()
    expect(visitContextWrites(firstEvaluate)).toEqual({
      writes: [
        ['context.review.summary', 'Blocking contract gap.'],
        ['context.review.required_changes', 'Preserve effort strings.'],
      ],
      systemCount: 2,
    })
    expect(visits[5].reason).toBeNull()
  })

  it('marks a loop-back on a succeeded visit when the run returns to an earlier node without failing', () => {
    // Modelled on resolution-program: Review routes back to Resolve on success.
    const journal = [
      ...stage('diagnose', 0), ...stage('evaluate', 1),
      ...stage('diagnose', 2), ...stage('evaluate', 3), ...stage('done', 4),
    ]
    const visits = build({ journal })

    expect(summarize(visits)).toEqual([
      'Diagnose 1/2 succeeded',
      'Evaluate 1/2 succeeded ↩',
      'Diagnose 2/2 succeeded',
      'Evaluate 2/2 succeeded',
      'Done 1/1 succeeded',
    ])
    expect(visits[1].next?.loopBack).toBe(true)
    expect(visits[3].next?.loopBack).toBe(false)
  })

  it('marks an unfinished visit interrupted once the run has ended, and the last failure as the run failure', () => {
    const journal = [...stage('start', 0), started('evaluate', 1)]
    expect(summarize(build({ journal, runStatus: 'canceled' }))).toEqual([
      'Start 1/1 succeeded',
      'Evaluate 1/1 interrupted',
    ])
    expect(summarize(build({ journal, runStatus: 'running' }))[1]).toBe('Evaluate 1/1 running')

    const failedRun = build({
      journal: [...stage('start', 0), started('validate', 1), failed('validate', 1, 'tool command failed with code 1')],
      runStatus: 'failed',
      executions: [execution('validate', 1, { outcome: 'fail', failure_reason: 'tool command failed with code 1', notes: '' })],
    })
    expect(failedRun[1].outcome).toBe('failed')
    expect(failedRun[1].reason).toBe('tool command failed with code 1')
  })

  it('shows a running visit whose node has a pending question as waiting', () => {
    const journal = [...stage('start', 0), started('ask', 1)]
    const visits = build({ journal, runStatus: 'running', waitingNodeIds: ['ask'] })
    expect(visits[1].outcome).toBe('waiting')
    expect(visitMark(visits[1])).toBe('waiting')
  })

  it('links a subflow visit to the child run it started', () => {
    const journal = [
      started('implement', 0),
      event('ChildRunStarted', { parent_node_id: 'implement', child_run_id: 'child-1', child_flow_name: 'Implement Task' }),
      ...stage('build', 0).map((entry) => ({ ...entry, sourceScope: 'child' as const })),
      event('ChildRunCompleted', { parent_node_id: 'implement', child_run_id: 'child-1', outcome: 'success' }),
      completed('implement', 0, { notes: 'Child completed' }),
    ]
    const visits = build({
      journal,
      childRuns: [{
        run_id: 'child-1',
        executions: [{ run_id: 'child-1', node_id: 'report', stage_index: 3, attempt: 0, status: { outcome: 'success', notes: 'Implemented the change.' } }],
      }],
    })
    expect(visits).toHaveLength(1)
    expect(visits[0].childRun).toMatchObject({
      runId: 'child-1',
      flowName: 'Implement Task',
      outcome: 'success',
      summary: 'Implemented the change.',
    })
    expect(visits[0].childRun?.endedAt).not.toBeNull()
    // The runtime's "Child completed" note is not a reason; the child summary says what happened.
    expect(visits[0].reason).toBeNull()
  })

  it('makes visits only of executions the journal started or the flow declares', () => {
    const visits = build({
      journal: [...stage('start', 0)],
      executions: [
        execution('start', 0, { outcome: 'success' }),
        // The post-run result summary: stage 0, never in the journal, not a flow node.
        execution('result_summary', 0, { outcome: 'success', notes: 'Summary.' }),
        // An exit node that ran without stage events is still a visit.
        execution('done', 1, { outcome: 'success' }),
      ],
    })
    expect(visits.map((visit) => visit.label)).toEqual(['Start', 'Done'])
  })

  it('nests parallel branches under their fan-out visit', () => {
    const journal = [
      started('fan_out', 0),
      event('ParallelBranchStarted', { node_id: 'fan_out', branch: 'lint', index: 0 }),
      event('ParallelBranchStarted', { node_id: 'fan_out', branch: 'typecheck', index: 1 }),
      event('ParallelBranchCompleted', { node_id: 'fan_out', branch: 'typecheck', index: 1, success: false }),
      event('ParallelBranchCompleted', { node_id: 'fan_out', branch: 'lint', index: 0, success: true }),
      completed('fan_out', 0),
      ...stage('done', 1),
    ]
    const visits = build({ journal })
    expect(visits.map((visit) => [visit.label, visit.parentKey === null ? 0 : 1, visit.outcome])).toEqual([
      ['Fan out', 0, 'succeeded'],
      ['Lint', 1, 'succeeded'],
      ['Typecheck', 1, 'did_not_pass'],
      ['Done', 0, 'succeeded'],
    ])
    expect(visits[1].parentKey).toBe(visits[0].key)
    expect(visits[0].next?.key).toBe(visits[3].key)
  })

  it('adds a visit as live data arrives and then completes it', () => {
    const journal = [...stage('start', 0), ...stage('evaluate', 1, 'fail')]
    const before = build({ journal, runStatus: 'running' })
    expect(summarize(before)).toEqual(['Start 1/1 succeeded', 'Evaluate 1/1 did_not_pass'])

    const withStart = [...journal, started('evaluate', 2)]
    const running = build({ journal: withStart, runStatus: 'running' })
    expect(summarize(running)).toEqual([
      'Start 1/1 succeeded',
      'Evaluate 1/2 did_not_pass ↩',
      'Evaluate 2/2 running',
    ])
    expect(running[2].endedAt).toBeNull()

    const finished = build({
      journal: [...withStart, completed('evaluate', 2)],
      runStatus: 'running',
      executions: [execution('evaluate', 2, { outcome: 'success', notes: 'Approved.', context_updates: { 'context.review.summary': null } })],
    })
    expect(summarize(finished)[2]).toBe('Evaluate 2/2 succeeded')
    expect(finished[2].reason).toBe('Approved.')
    expect(visitContextWrites(finished[2]).writes).toEqual([['context.review.summary', null]])
  })

  it('keeps visits known only from the executions list', () => {
    const visits = build({ executions: [execution('done', 7, { outcome: 'success' })] })
    expect(summarize(visits)).toEqual(['Done 1/1 succeeded'])
    expect(visits[0].startedAt).toBeNull()
  })
})

describe('visit transcript', () => {
  const segment = (overrides: Partial<RunTranscriptSegment>): RunTranscriptSegment => ({
    id: 'segment-1', turn_id: 'turn', order: 1, kind: 'assistant_message', role: 'assistant', status: 'complete',
    timestamp: '2026-09-27T17:00:00Z', updated_at: '2026-09-27T17:00:00Z', content: 'Checking.',
    node_id: 'evaluate', stage_index: 2, attempt: 0, latest_sequence: 1, source_scope: 'root',
    source_flow_name: null, source_parent_node_id: null, source_run_id: RUN,
    ...overrides,
  })

  it('keeps this visit rows, drops the final status envelope, and finds the prompt', () => {
    const [visit] = build({ journal: stage('evaluate', 2) })
    const rows = visitTranscriptRows(visit, [
      segment({ id: 'a', order: 1 }),
      segment({ id: 'b', order: 2, content: '{"outcome":"fail","context_updates":{}}' }),
      segment({ id: 'other-visit', stage_index: 4 }),
    ])
    expect(rows.map((row) => row.segment.id)).toEqual(['a'])
    expect(visitPrompt(visit, [
      { run_id: RUN, node_id: 'evaluate', stage_index: 4, attempt: 0, content: 'Other.' },
      { run_id: RUN, node_id: 'evaluate', stage_index: 2, attempt: 0, content: 'Judge the worktree.' },
    ])).toBe('Judge the worktree.')
    expect(flowNodes.evaluate.readsContext).toEqual(['context.workspace.path', 'context.task.objective'])
    expect(flowNodes.validate.command).toBe('just test')
  })
})
