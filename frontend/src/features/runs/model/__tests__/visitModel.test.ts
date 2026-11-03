import { describe, expect, it } from 'vitest'

import type { NodeExecutionResponse, RunTranscriptSegment } from '@/lib/api/attractorApi'
import type { TimelineEventEntry } from '../shared'
import {
  buildRunVisits,
  formatVisitErrorKind,
  parallelBranchResults,
  visitContextWrites,
  visitFlowNodesFromSnapshot,
  visitMark,
  visitPrompt,
  visitTranscriptEntries,
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
  event('StageCompleted', { node_id: node, index, outcome: 'completed', decision: 'done', ...extra })
)
const failed = (node: string, index: number, error = 'stage_failed') => (
  event('StageFailed', { node_id: node, index, error, will_retry: false })
)
// A stage completes with a decision ('done' unless named) or ends in error.
const stage = (node: string, index: number, end: string = 'done') => (
  [started(node, index), end === 'error' ? failed(node, index) : completed(node, index, { decision: end })]
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
  `${visit.label} ${visit.number}/${visit.count} ${visit.outcome}${visit.decision ? ` ${visit.decision}` : ''}${visit.next?.loopBack ? ' ↩' : ''}`
))

describe('buildRunVisits', () => {
  it('follows a review loop where Evaluate ran four times and two rejections routed back to Implement', () => {
    // A rejection is a completed visit whose decision routes back; a tolerated error is an error.
    const journal = [
      ...stage('start', 0),
      ...stage('implement', 1), ...stage('evaluate', 2, 'revise'),
      ...stage('implement', 3), ...stage('evaluate', 4, 'approve'),
      ...stage('validate', 5, 'error'), ...stage('diagnose', 6),
      ...stage('implement', 7), ...stage('evaluate', 8, 'revise'),
      ...stage('implement', 9), ...stage('evaluate', 10, 'approve'),
      ...stage('validate', 11), ...stage('done', 12),
    ]
    const rejection = {
      status: 'completed',
      decision: 'revise',
      failure_reason: '',
      notes: '',
      context_updates: {
        'context.review.summary': 'Blocking contract gap.',
        'context.review.required_changes': 'Preserve effort strings.',
        last_stage: 'evaluate',
        last_response: '{}',
      },
    }
    const toleratedError = { status: 'error', decision: null, failure_kind: 'runtime', tolerable: true, failure_reason: 'tool command failed with code 1' }
    const visits = build({ journal, executions: [execution('evaluate', 2, rejection), execution('validate', 5, toleratedError)] })

    expect(summarize(visits)).toEqual([
      'Start 1/1 completed done',
      'Implement 1/4 completed done',
      'Evaluate 1/4 completed revise ↩',
      'Implement 2/4 completed done',
      'Evaluate 2/4 completed approve',
      'Validate 1/2 error',
      'Diagnose 1/1 completed done ↩',
      'Implement 3/4 completed done',
      'Evaluate 3/4 completed revise ↩',
      'Implement 4/4 completed done',
      'Evaluate 4/4 completed approve',
      'Validate 2/2 completed done',
      'Done 1/1 completed done',
    ])
    const firstEvaluate = visits[2]
    expect(visitMark(firstEvaluate)).toBe('loop_back')
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
    // The error's kind and tolerance stay separate from the status and the (absent) decision.
    expect(visits[5]).toMatchObject({ outcome: 'error', decision: null, failureKind: 'runtime', tolerable: true, reason: 'tool command failed with code 1' })
    expect(visitMark(visits[5])).toBe('error')
    expect(formatVisitErrorKind(visits[5])).toBe('runtime error, tolerable')
    expect(formatVisitErrorKind(visits[4])).toBeNull()
  })

  it('marks a loop-back on a completed visit when the run returns to an earlier node without failing', () => {
    // Modelled on resolution-program: Review routes back to Resolve on success.
    const journal = [
      ...stage('diagnose', 0), ...stage('evaluate', 1),
      ...stage('diagnose', 2), ...stage('evaluate', 3), ...stage('done', 4),
    ]
    const visits = build({ journal })

    expect(summarize(visits)).toEqual([
      'Diagnose 1/2 completed done',
      'Evaluate 1/2 completed done ↩',
      'Diagnose 2/2 completed done',
      'Evaluate 2/2 completed done',
      'Done 1/1 completed done',
    ])
    expect(visits[1].next?.loopBack).toBe(true)
    expect(visits[3].next?.loopBack).toBe(false)
  })

  it('marks an unfinished visit interrupted once the run has ended, and the last failure as the run failure', () => {
    const journal = [...stage('start', 0), started('evaluate', 1)]
    expect(summarize(build({ journal, runStatus: 'canceled' }))).toEqual([
      'Start 1/1 completed done',
      'Evaluate 1/1 interrupted',
    ])
    expect(summarize(build({ journal, runStatus: 'running' }))[1]).toBe('Evaluate 1/1 running')

    // A node stopped by cancel ends when the run does.
    const runEnd = event('PipelineFailed', { error: 'aborted_by_user' })
    const canceled = build({ journal: [...journal, runEnd], runStatus: 'canceled' })
    expect(canceled[1].endedAt).toBe(runEnd.receivedAt)

    const failedRun = build({
      journal: [...stage('start', 0), started('validate', 1), failed('validate', 1, 'tool command failed with code 1')],
      runStatus: 'failed',
      executions: [execution('validate', 1, { status: 'error', failure_kind: 'runtime', tolerable: false, failure_reason: 'tool command failed with code 1', notes: '' })],
    })
    expect(failedRun[1].outcome).toBe('failed')
    expect(failedRun[1].failureKind).toBe('runtime')
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
        executions: [{ run_id: 'child-1', node_id: 'report', stage_index: 3, attempt: 0, status: { status: 'completed', decision: 'done', notes: 'Implemented the change.' } }],
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
        execution('start', 0, { status: 'completed', decision: 'next' }),
        // The post-run result summary: stage 0, never in the journal, not a flow node.
        execution('result_summary', 0, { status: 'completed', decision: 'done', notes: 'Summary.' }),
        // An exit node that ran without stage events is still a visit.
        execution('done', 1, { status: 'completed', decision: 'done' }),
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
      ['Fan out', 0, 'completed'],
      ['Lint', 1, 'completed'],
      ['Typecheck', 1, 'error'],
      ['Done', 0, 'completed'],
    ])
    expect(visits[1].parentKey).toBe(visits[0].key)
    expect(visits[0].next?.key).toBe(visits[3].key)
  })

  it('gives each node a parallel branch runs its own visit', () => {
    const branchNode = (type: string, node: string, extra: Record<string, unknown> = {}) => (
      event(type, { node_id: 'fan_out', branch: 'first', index: 0, branch_node_id: node, ...extra })
    )
    const journal = [
      started('fan_out', 0),
      event('ParallelBranchStarted', { node_id: 'fan_out', branch: 'first', index: 0 }),
      branchNode('ParallelBranchNodeStarted', 'first'),
      branchNode('ParallelBranchNodeCompleted', 'first', { outcome: 'completed' }),
      branchNode('ParallelBranchNodeStarted', 'second'),
    ]
    const running = build({ journal, runStatus: 'running' })
    // Only the node the branch is on now is running, so only it can be steered.
    expect(running.map((visit) => [visit.label, visit.parentKey === running[0].key, visit.outcome])).toEqual([
      ['Fan out', false, 'running'],
      ['First', true, 'completed'],
      ['Second', true, 'running'],
    ])

    const done = build({
      journal: [
        ...journal,
        branchNode('ParallelBranchNodeCompleted', 'second', { outcome: 'error' }),
        event('ParallelBranchCompleted', { node_id: 'fan_out', branch: 'first', index: 0, success: false }),
        completed('fan_out', 0),
      ],
      runStatus: 'completed',
    })
    expect(done.map((visit) => visit.outcome)).toEqual(['completed', 'completed', 'error'])
  })

  it('adds a visit as live data arrives and then completes it', () => {
    const journal = [...stage('start', 0), ...stage('evaluate', 1, 'error')]
    const before = build({ journal, runStatus: 'running' })
    expect(summarize(before)).toEqual(['Start 1/1 completed done', 'Evaluate 1/1 error'])

    const withStart = [...journal, started('evaluate', 2)]
    const running = build({ journal: withStart, runStatus: 'running' })
    expect(summarize(running)).toEqual([
      'Start 1/1 completed done',
      'Evaluate 1/2 error ↩',
      'Evaluate 2/2 running',
    ])
    expect(running[2].endedAt).toBeNull()

    const finished = build({
      journal: [...withStart, completed('evaluate', 2)],
      runStatus: 'running',
      executions: [execution('evaluate', 2, { status: 'completed', decision: 'approve', notes: 'Approved.', context_updates: { 'context.review.summary': null } })],
    })
    // The execution's status.json decision wins over the journal's.
    expect(summarize(finished)[2]).toBe('Evaluate 2/2 completed approve')
    expect(finished[2].reason).toBe('Approved.')
    expect(visitContextWrites(finished[2]).writes).toEqual([['context.review.summary', null]])
  })

  it('reads each declared branch result, with failed, canceled and terminal-result evidence', () => {
    const results = [
      { id: 'lint', index: 0, status: 'completed', started: true, context: {}, route_trace: ['lint'], decisions: [{ node: 'lint', decision: 'done', target: 'end' }], notes: 'clean' },
      { id: 'typecheck', index: 1, status: 'error', started: true, context: {}, route_trace: ['typecheck'], decisions: [], notes: '', error: { node: 'typecheck', kind: 'runtime', tolerable: false, reason: 'tsc exited 2', attempts: 3 } },
      { id: 'review', index: 2, status: 'terminal_result', started: true, context: {}, route_trace: ['review'], decisions: [], notes: '', terminal: { exit_id: 'stop', result: { name: 'blocked', outcome: 'failure' }, branch: 'review', branch_index: 2, node: 'review', decision: 'block', notes: '' } },
      { id: 'docs', index: 3, status: 'canceled', started: false, context: {}, route_trace: [], decisions: [], notes: '' },
    ]
    const [fanOut] = build({
      journal: stage('fan_out', 0),
      executions: [execution('fan_out', 0, { status: 'error', failure_kind: 'runtime', tolerable: false, context_updates: { 'parallel.results': results } })],
    })
    expect(parallelBranchResults(fanOut)).toEqual([
      { id: 'lint', index: 0, status: 'completed', decisions: [{ node: 'lint', decision: 'done', target: 'end' }], notes: 'clean', error: null, terminal: null },
      { id: 'typecheck', index: 1, status: 'error', decisions: [], notes: null, error: { node: 'typecheck', kind: 'runtime', tolerable: false, reason: 'tsc exited 2', attempts: 3 }, terminal: null },
      { id: 'review', index: 2, status: 'terminal_result', decisions: [], notes: null, error: null, terminal: { exitId: 'stop', name: 'blocked', outcome: 'failure', node: 'review' } },
      { id: 'docs', index: 3, status: 'canceled', decisions: [], notes: null, error: null, terminal: null },
    ])
    expect(parallelBranchResults({ status: { context_updates: {} } })).toEqual([])
  })

  it('keeps visits known only from the executions list', () => {
    const visits = build({ executions: [execution('done', 7, { status: 'completed', decision: 'done' })] })
    expect(summarize(visits)).toEqual(['Done 1/1 completed done'])
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
    const entries = visitTranscriptEntries(visit, [
      segment({ id: 'a', order: 1 }),
      segment({ id: 'b', order: 2, content: '{"decision":"revise","context_updates":{}}' }),
      segment({ id: 'c', order: 3, kind: 'plan', content: '```json\n{"decision":"done"}\n```' }),
      segment({ id: 'd', order: 4, kind: 'user_message', role: 'user', content: '{"decision":"done"}' }),
      segment({ id: 'other-visit', stage_index: 4 }),
    ])
    expect(entries.map((entry) => entry.id)).toEqual(['a'])
    expect(visitPrompt(visit, [
      { run_id: RUN, node_id: 'evaluate', stage_index: 4, attempt: 0, content: 'Other.' },
      { run_id: RUN, node_id: 'evaluate', stage_index: 2, attempt: 0, content: 'Judge the worktree.' },
    ])).toBe('Judge the worktree.')
    expect(flowNodes.evaluate.readsContext).toEqual(['context.workspace.path', 'context.task.objective'])
    expect(flowNodes.validate.command).toBe('just test')
  })
})
