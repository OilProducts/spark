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
  visitToolResult,
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

const execution = (node: string, index: number, status: Record<string, unknown> | null, attempt = 0): NodeExecutionResponse => ({
  run_id: RUN, node_id: node, stage_index: index, attempt, status,
})

// A parallel branch's events, as the runtime emits them: the node id is the fan-out's,
// the index the branch's, and node events name the invocation they report.
const branchStarted = (branch: string, index = 0) => event('ParallelBranchStarted', { node_id: 'fan_out', branch, index })
const branchCompleted = (branch: string, index = 0, success = true) => (
  event('ParallelBranchCompleted', { node_id: 'fan_out', branch, index, success })
)
const nodeStarted = (node: string, stageIndex: number, attempt: number, index = 0, branch = node) => event('ParallelBranchNodeStarted', {
  node_id: 'fan_out', branch, index, branch_node_id: node, stage_index: stageIndex, attempt,
})
const nodeCompleted = (node: string, stageIndex: number, attempt: number, outcome = 'completed', index = 0, branch = node) => event('ParallelBranchNodeCompleted', {
  node_id: 'fan_out', branch, index, branch_node_id: node, stage_index: stageIndex, attempt, outcome,
})
const nested = (visits: RunVisit[]) => visits.filter((visit) => visit.parentKey !== null)

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

  it('nests parallel branches under their fan-out visit, one visit per invocation with its execution', () => {
    const journal = [
      started('fan_out', 0),
      branchStarted('lint', 0),
      branchStarted('typecheck', 1),
      nodeStarted('lint', 0, 0, 0),
      nodeStarted('typecheck', 0, 0, 1),
      nodeCompleted('typecheck', 0, 0, 'error', 1),
      branchCompleted('typecheck', 1, false),
      nodeCompleted('lint', 0, 0, 'completed', 0),
      branchCompleted('lint', 0),
      completed('fan_out', 0),
      ...stage('done', 1),
    ]
    const executions = [
      execution('fan_out', 0, { status: 'completed', decision: 'next' }),
      execution('lint', 0, {
        status: 'completed',
        decision: 'clean',
        notes: 'No lint.',
        context_updates: { 'context.lint.status': 'clean', 'context.tool.output': 'ok', 'context.tool.exit_code': 0 },
      }),
      execution('typecheck', 0, { status: 'error', failure_kind: 'runtime', tolerable: false, failure_reason: 'tsc exited 2' }),
      execution('done', 1, { status: 'completed', decision: 'done' }),
    ]
    const visits = build({ journal, executions })
    expect(visits.map((visit) => [visit.label, visit.parentKey === null ? 0 : 1, visit.outcome, visit.number, visit.count])).toEqual([
      ['Fan out', 0, 'completed', 1, 1],
      ['Lint', 1, 'completed', 1, 1],
      ['Typecheck', 1, 'error', 1, 1],
      ['Done', 0, 'completed', 1, 1],
    ])
    const [fanOut, lint, typecheck, done] = visits
    expect(lint.parentKey).toBe(fanOut.key)
    expect(fanOut.next?.key).toBe(done.key)
    // A branch visit has the journal's timing and the execution's status together.
    expect(lint).toMatchObject({ decision: 'clean', reason: 'No lint.', next: null, stageIndex: 0, attempt: 0, executionIdentity: { stageIndex: 0, attempt: 0 } })
    expect(lint.startedAt).toBe(journal[1].receivedAt)
    expect(lint.endedAt).toBe(journal[7].receivedAt)
    expect(visitContextWrites(lint).writes).toEqual([['context.lint.status', 'clean']])
    expect(visitToolResult(lint)).toEqual({ output: 'ok', exitCode: 0 })
    expect(typecheck).toMatchObject({ failureKind: 'runtime', tolerable: false, reason: 'tsc exited 2', next: null })
    expect(fanOut.executionIdentity).toBeUndefined()
  })

  it('reaches the same visits whichever of the journal and executions arrives first', () => {
    const journal = [
      started('fan_out', 0),
      branchStarted('lint'),
      nodeStarted('lint', 0, 0),
      nodeCompleted('lint', 0, 0),
      branchCompleted('lint'),
      completed('fan_out', 0),
      ...stage('done', 1),
    ]
    const executions = [execution('lint', 0, { status: 'completed', decision: 'clean' })]
    const final = build({ journal, executions })

    // Executions first: known only from the executions list, the execution stands alone.
    expect(summarize(build({ executions }))).toEqual(['Lint 1/1 completed clean'])
    // Journal first: the rows and their keys are already final, so a selection survives.
    const journalOnly = build({ journal })
    expect(journalOnly.map((visit) => visit.key)).toEqual(final.map((visit) => visit.key))
    expect(journalOnly[1]).toMatchObject({ outcome: 'completed', decision: null, status: null })
    expect(summarize(final)).toEqual(['Fan out 1/1 completed done', 'Lint 1/1 completed clean', 'Done 1/1 completed done'])
    expect(final[1].startedAt).toBe(journalOnly[1].startedAt)
    expect(final[1].endedAt).toBe(journalOnly[1].endedAt)
  })

  it('keeps a branch without node events as one journal-only visit that claims no execution', () => {
    const journal = [
      started('fan_out', 0),
      branchStarted('lint', 0),
      branchStarted('typecheck', 1),
      branchCompleted('typecheck', 1, false),
      branchCompleted('lint', 0, true),
      completed('fan_out', 0),
      ...stage('done', 1),
    ]
    const [, lint, typecheck, done] = build({ journal })
    expect(lint).toMatchObject({ nodeId: 'lint', outcome: 'completed', executionIdentity: null })
    expect(typecheck).toMatchObject({ nodeId: 'typecheck', outcome: 'error', executionIdentity: null })
    expect(done.label).toBe('Done')
    expect(lint.endedAt).not.toBeNull()

    // The parent's stage and attempt are not the branch's: no execution content matches.
    const records = { run_id: RUN, node_id: 'lint', stage_index: 0, attempt: 0 }
    expect(visitPrompt(lint, [{ ...records, content: 'Not known to be this visit.' }])).toBeNull()
    expect(build({
      journal,
      executions: [execution('lint', 0, { status: 'completed', decision: 'clean' })],
    }).map((visit) => [visit.label, visit.parentKey === null, visit.decision])).toEqual([
      ['Fan out', true, 'done'],
      ['Lint', false, null],
      ['Typecheck', false, null],
      // Unmatched, the execution keeps its own row.
      ['Lint', true, 'clean'],
      ['Done', true, 'done'],
    ])
  })

  it('gives each node a parallel branch runs its own visit', () => {
    const journal = [
      started('fan_out', 0),
      branchStarted('first'),
      nodeStarted('first', 0, 0),
      nodeCompleted('first', 0, 0),
      nodeStarted('second', 0, 1, 0, 'first'),
    ]
    const running = build({ journal, runStatus: 'running' })
    // Only the node the branch is on now is running, so only it can be steered.
    expect(running.map((visit) => [visit.label, visit.parentKey === running[0].key, visit.outcome])).toEqual([
      ['Fan out', false, 'running'],
      ['First', true, 'completed'],
      ['Second', true, 'running'],
    ])
    expect(running[2].startedAt).toBe(journal[4].receivedAt)

    const done = build({
      journal: [
        ...journal,
        nodeCompleted('second', 0, 1, 'error', 0, 'first'),
        branchCompleted('first', 0, false),
        completed('fan_out', 0),
      ],
      runStatus: 'completed',
      executions: [
        execution('first', 0, { status: 'completed', decision: 'go' }),
        execution('second', 0, { status: 'error', failure_kind: 'contract', failure_reason: 'bad' }, 1),
      ],
    })
    expect(done.map((visit) => visit.outcome)).toEqual(['completed', 'completed', 'error'])
    expect(done[1].decision).toBe('go')
    expect(done[2]).toMatchObject({ reason: 'bad', stageIndex: 0, attempt: 1 })
    expect(new Set(done.map((visit) => visit.key)).size).toBe(3)
  })

  it('closes the open visit when its branch ends without a node completion', () => {
    const journal = [
      started('fan_out', 0),
      branchStarted('first'),
      nodeStarted('first', 0, 0),
      branchCompleted('first', 0, false),
    ]
    const [, first] = build({ journal, runStatus: 'running' })
    expect(first).toMatchObject({ outcome: 'error', endedAt: journal[3].receivedAt })
  })

  it('gives a same-node branch retry its own visit and execution', () => {
    const journal = [
      started('fan_out', 1),
      branchStarted('lint'),
      nodeStarted('lint', 1, 0),
      nodeCompleted('lint', 1, 0, 'error'),
      nodeStarted('lint', 1, 1),
      nodeCompleted('lint', 1, 1),
      branchCompleted('lint'),
      completed('fan_out', 1),
    ]
    const visits = build({
      journal,
      executions: [
        execution('lint', 1, { status: 'error', failure_kind: 'runtime', failure_reason: 'provider down' }),
        execution('lint', 1, { status: 'completed', decision: 'clean' }, 1),
      ],
    })
    expect(summarize(visits)).toEqual(['Fan out 1/1 completed done', 'Lint 1/2 error', 'Lint 2/2 completed clean'])
    expect(visits[1]).toMatchObject({ attempt: 0, reason: 'provider down', endedAt: journal[3].receivedAt })
    expect(visits[2]).toMatchObject({ attempt: 1, startedAt: journal[4].receivedAt, endedAt: journal[5].receivedAt })
  })

  it('keeps each fan-out invocation\'s branches apart, including under a nonzero fork attempt', () => {
    const lintRun = (stageIndex: number, attempt: number) => [
      branchStarted('lint'),
      nodeStarted('lint', stageIndex, attempt),
      nodeCompleted('lint', stageIndex, attempt),
      branchCompleted('lint'),
    ]
    const journal = [
      started('fan_out', 1),
      ...lintRun(1, 0),
      failed('fan_out', 1),
      started('fan_out', 1),
      ...lintRun(1, 1_000_000),
      completed('fan_out', 1),
    ]
    const executions = [
      execution('lint', 1, { status: 'completed', decision: 'first' }),
      execution('lint', 1, { status: 'completed', decision: 'second' }, 1_000_000),
    ]
    const visits = build({ journal, executions })
    expect(summarize(visits)).toEqual([
      'Fan out 1/2 error ↩',
      'Lint 1/2 completed first',
      'Fan out 2/2 completed done',
      'Lint 2/2 completed second',
    ])
    expect(visits[1].parentKey).toBe(visits[0].key)
    expect(visits[3].parentKey).toBe(visits[2].key)
    expect(visits[3]).toMatchObject({ stageIndex: 1, attempt: 1_000_000, executionIdentity: { stageIndex: 1, attempt: 1_000_000 } })
    expect(visits[0].next?.key).toBe(visits[2].key)
  })

  it('keeps a root invocation of a node a branch also ran', () => {
    const journal = [
      started('fan_out', 1),
      branchStarted('lint'),
      nodeStarted('lint', 1, 0),
      nodeCompleted('lint', 1, 0),
      branchCompleted('lint'),
      completed('fan_out', 1),
      ...stage('lint', 2),
      ...stage('done', 3),
    ]
    const visits = build({
      journal,
      executions: [
        execution('lint', 1, { status: 'completed', decision: 'branch' }),
        execution('lint', 2, { status: 'completed', decision: 'root' }),
      ],
    })
    expect(summarize(visits)).toEqual([
      'Fan out 1/1 completed done',
      'Lint 1/2 completed branch',
      'Lint 2/2 completed root',
      'Done 1/1 completed done',
    ])
    expect(visits[0].next?.key).toBe(visits[2].key)
    expect(visits[2].executionIdentity).toBeUndefined()
  })

  it('claims no execution when concurrent branches share its identity', () => {
    const journal = [
      started('fan_out', 1),
      branchStarted('lint', 0),
      branchStarted('lint', 1),
      nodeStarted('lint', 1, 0, 0),
      nodeStarted('lint', 1, 0, 1),
      nodeCompleted('lint', 1, 0, 'completed', 0),
      nodeCompleted('lint', 1, 0, 'error', 1),
      branchCompleted('lint', 0),
      branchCompleted('lint', 1, false),
      completed('fan_out', 1),
    ]
    const visits = build({ journal, executions: [execution('lint', 1, { status: 'completed', decision: 'shared', notes: 'One record.' })] })
    expect(visits.map((visit) => [visit.label, visit.parentKey === null, visit.outcome, visit.decision])).toEqual([
      ['Fan out', true, 'completed', 'done'],
      ['Lint', false, 'completed', null],
      ['Lint', false, 'error', null],
      // The record stays reachable on its own row rather than on either branch.
      ['Lint', true, 'completed', 'shared'],
    ])
    const prompts = [{ run_id: RUN, node_id: 'lint', stage_index: 1, attempt: 0, content: 'Shared.' }]
    for (const branch of nested(visits)) {
      expect(branch.executionIdentity).toBeNull()
      expect(branch.status).toBeNull()
      expect(visitPrompt(branch, prompts)).toBeNull()
      expect(visitContextWrites(branch).writes).toEqual([])
    }
    expect(visitPrompt(visits[3], prompts)).toBe('Shared.')
    // Before any execution arrives neither branch can claim content either.
    expect(nested(build({ journal })).map((visit) => visit.executionIdentity)).toEqual([null, null])
  })

  it('keeps a branch\'s visits, timing and numbering through relaunches', () => {
    // Before the first node: no second provisional visit, and the first timestamp stays.
    const beforeNode = [started('fan_out', 1), branchStarted('first'), branchStarted('first')]
    const provisional = nested(build({ journal: beforeNode, runStatus: 'running' }))
    expect(provisional).toHaveLength(1)
    expect(provisional[0]).toMatchObject({ startedAt: beforeNode[1].receivedAt, outcome: 'running', executionIdentity: null })

    // After a node completed, and while a later one is open.
    const journal = [
      ...beforeNode,
      nodeStarted('first', 1, 0),
      nodeCompleted('first', 1, 0),
      branchStarted('first'),
      nodeStarted('second', 1, 1, 0, 'first'),
      branchStarted('first'),
    ]
    const running = build({ journal, runStatus: 'running' })
    expect(summarize(running)).toEqual(['Fan out 1/1 running', 'First 1/1 completed', 'Second 1/1 running'])
    // A confirmed first visit keeps the branch's first start, its key and its end.
    expect(running[1]).toMatchObject({ startedAt: beforeNode[1].receivedAt, endedAt: journal[4].receivedAt })
    expect(running[2]).toMatchObject({ startedAt: journal[6].receivedAt, endedAt: null })
    expect(running[1].key).toBe(provisional[0].key)
  })

  it('confirms a provisional visit as the node a relaunched branch actually reports', () => {
    const journal = [started('fan_out', 1), branchStarted('first'), nodeStarted('second', 1, 3, 0, 'first')]
    const [, visit, ...rest] = build({ journal, runStatus: 'running' })
    expect(rest).toEqual([])
    expect(visit).toMatchObject({ nodeId: 'second', label: 'Second', attempt: 3, startedAt: journal[1].receivedAt })
  })

  it('shows one visit for an accepted result replayed after a resume', () => {
    // The real sequence: the resume restarts the fork's stage and relaunches its branch, whose
    // accepted result replays under its original identity before the next invocation.
    const first = [
      ...stage('start', 0),
      started('fan_out', 1),
      branchStarted('first'),
      nodeStarted('first', 1, 0),
      nodeCompleted('first', 1, 0),
    ]
    const replay = () => [
      started('fan_out', 1),
      branchStarted('first'),
      nodeStarted('first', 1, 0),
      nodeCompleted('first', 1, 0),
    ]
    const journal = [
      ...first,
      ...replay(),
      ...replay(),
      nodeStarted('second', 1, 1, 0, 'first'),
      nodeCompleted('second', 1, 1, 'completed', 0, 'first'),
      branchCompleted('first'),
      completed('fan_out', 1),
      ...stage('done', 2),
    ]
    const executions = [
      execution('fan_out', 1, { status: 'completed', decision: 'next' }),
      execution('first', 1, { status: 'completed', decision: 'accepted' }),
      execution('second', 1, { status: 'completed', decision: 'after' }, 1),
    ]
    const visits = build({ journal, executions })
    expect(summarize(visits)).toEqual([
      'Start 1/1 completed done',
      'Fan out 1/1 completed next',
      'First 1/1 completed accepted',
      'Second 1/1 completed after',
      'Done 1/1 completed done',
    ])
    expect(visits[1].next?.key).toBe(visits[4].key)
    // The replays neither moved nor extended the original invocation.
    expect(visits[2]).toMatchObject({ startedAt: first[3].receivedAt, endedAt: first[5].receivedAt, attempt: 0 })
    expect(visits[3]).toMatchObject({ attempt: 1 })
    expect(visitPrompt(visits[3], [
      { run_id: RUN, node_id: 'second', stage_index: 1, attempt: 1, content: 'Second instructions.' },
    ])).toBe('Second instructions.')
  })

  it('reuses an interrupted invocation still open when its identity starts again', () => {
    const journal = [
      started('fan_out', 1),
      branchStarted('first'),
      nodeStarted('first', 1, 0),
      started('fan_out', 1),
      branchStarted('first'),
      nodeStarted('first', 1, 0),
      nodeCompleted('first', 1, 0),
      nodeCompleted('first', 1, 0),
    ]
    const visits = build({ journal, runStatus: 'running' })
    expect(summarize(visits)).toEqual(['Fan out 1/1 running', 'First 1/1 completed'])
    expect(visits[1]).toMatchObject({ startedAt: journal[1].receivedAt, endedAt: journal[6].receivedAt })
  })

  it('counts a resumed stage once but an explicit retry of an interrupted stage as a new attempt', () => {
    const interrupted = [...stage('start', 0), started('evaluate', 1)]
    const resumed = build({ journal: [...interrupted, started('evaluate', 1), completed('evaluate', 1)] })
    expect(summarize(resumed)).toEqual(['Start 1/1 completed done', 'Evaluate 1/1 completed done'])
    expect(resumed[1]).toMatchObject({ attempt: 0, startedAt: interrupted[2].receivedAt })

    const retried = build({
      journal: [...interrupted, event('PipelineRetryStarted', { current_node: 'evaluate' }), started('evaluate', 1), completed('evaluate', 1)],
      executions: [execution('evaluate', 1, { status: 'completed', decision: 'done' }, 1)],
    })
    expect(retried.map((visit) => [visit.nodeId, visit.attempt])).toEqual([['start', 0], ['evaluate', 0], ['evaluate', 1]])
  })

  it('leaves a branch node event with no started branch to the execution fallback', () => {
    // A truncated journal: the node's events alone never make or shift a branch visit.
    const journal = [started('fan_out', 1), nodeStarted('lint', 1, 0), nodeCompleted('lint', 1, 0), completed('fan_out', 1)]
    const visits = build({ journal, executions: [execution('lint', 1, { status: 'completed', decision: 'clean' })] })
    expect(summarize(visits)).toEqual(['Fan out 1/1 completed done', 'Lint 1/1 completed clean'])
    expect(visits[1].parentKey).toBeNull()
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

  describe('of a parallel branch visit', () => {
    const journal = [
      started('fan_out', 1),
      branchStarted('evaluate'),
      nodeStarted('evaluate', 1, 2),
      nodeCompleted('evaluate', 1, 2),
    ]

    it('keeps only its own run, node, stage and attempt, with the transcript rules unchanged', () => {
      const [, branch] = build({ journal })
      const own = { node_id: 'evaluate', stage_index: 1, attempt: 2 }
      const entries = visitTranscriptEntries(branch, [
        segment({ id: 'own', ...own }),
        segment({ id: 'envelope', ...own, order: 2, content: '{"decision":"done"}' }),
        segment({ id: 'other-run', ...own, source_run_id: 'other' }),
        segment({ id: 'other-node', ...own, node_id: 'lint' }),
        segment({ id: 'other-stage', ...own, stage_index: 2 }),
        segment({ id: 'other-attempt', ...own, attempt: 1 }),
      ])
      expect(entries.map((entry) => entry.id)).toEqual(['own'])
      expect(visitPrompt(branch, [
        { run_id: RUN, ...own, attempt: 1, content: 'Other attempt.' },
        { run_id: 'other', ...own, content: 'Other run.' },
        { run_id: RUN, ...own, content: 'Its instructions.' },
      ])).toBe('Its instructions.')
    })

    it('finds nothing while the branch has reported no node', () => {
      const [, provisional] = build({ journal: journal.slice(0, 2), runStatus: 'running' })
      expect(visitTranscriptEntries(provisional, [
        segment({ id: 'parent-identity', stage_index: 1, attempt: 0 }),
      ])).toEqual([])
    })
  })
})
