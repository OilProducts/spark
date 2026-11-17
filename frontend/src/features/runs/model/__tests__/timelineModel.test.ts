import {
  buildGroupedPendingInterviewGates,
  buildPendingInterviewGates,
  journalEntriesShown,
  journalEntrySummary,
  toTimelineEvent,
} from '@/features/runs/model/timelineModel'
import { humanizeTimelineType } from '@/features/runs/model/shared'

describe('timelineModel', () => {
  it('appends outcome notes to stage-completed summaries', () => {
    const event = toTimelineEvent({
      raw_type: 'StageCompleted',
      kind: 'stage',
      sequence: 7,
      emitted_at: '2026-07-08T10:00:07Z',
      summary: 'Stage verify completed (decision done) — checksums ok',
      node_id: 'verify',
      stage_index: 3,
      source_scope: 'root',
      severity: 'info',
      id: 'journal-7',
      payload: { outcome: 'completed', decision: 'done', notes: 'checksums ok', node_id: 'verify' },
    })
    expect(event?.summary).toContain('checksums ok')
  })

  it('summarizes a live stage completion and a transition by decision, target and visit', () => {
    expect(toTimelineEvent({
      type: 'StageCompleted', sequence: 4, emitted_at: '2026-07-08T10:00:04Z',
      index: 2, node_id: 'review', outcome: 'completed', decision: 'revise', notes: 'needs a test',
    })).toMatchObject({ category: 'stage', severity: 'info', summary: 'review completed' })

    const transition = toTimelineEvent({
      type: 'StageTransition', sequence: 5, emitted_at: '2026-07-08T10:00:05Z',
      index: 2, node_id: 'review', decision: 'revise', target: 'implement', visit: 2,
    })
    expect(transition).toMatchObject({
      type: 'StageTransition', category: 'stage', severity: 'info', nodeId: 'review', stageIndex: 2,
      summary: 'review → implement on revise (visit 2)',
    })
    expect(humanizeTimelineType('StageTransition')).toBe('Transition')
    // A first visit needs no visit number.
    expect(toTimelineEvent({
      type: 'StageTransition', sequence: 6, emitted_at: '2026-07-08T10:00:06Z',
      index: 0, node_id: 'start', decision: 'next', target: 'review', visit: 1,
    })?.summary).toBe('start → review on next')
  })


  it('summarizes completed failure outcomes without classifying them as runtime errors', () => {
    const event = toTimelineEvent({
      type: 'PipelineCompleted',
      sequence: 3,
      emitted_at: '2026-04-06T12:00:00Z',
      node_id: 'done',
      outcome: 'failure',
      outcome_reason_message: 'Goal gate not satisfied',
    })

    expect(event).toMatchObject({
      type: 'PipelineCompleted',
      severity: 'warning',
      summary: 'Run completed at done (failure: Goal gate not satisfied)',
    })
  })

  it('names steps in journal lines by their flow labels and keeps the runtime wording for other kinds', () => {
    const labels: Record<string, string> = { run_tests: 'Run Test Suite', draft_notes: 'Draft Notes', run_milestone: 'Run Milestone' }
    const nodeLabel = (nodeId: string, scope: string) => (scope === 'root' ? labels[nodeId] : null) ?? nodeId
    const line = (rawType: string, payload: Record<string, unknown>, extra: Record<string, unknown> = {}) => journalEntrySummary(toTimelineEvent({
      id: 'journal-1', sequence: 1, emitted_at: '2026-07-08T10:00:00Z', kind: 'stage', raw_type: rawType,
      summary: `server ${rawType}`, severity: 'info', source_scope: 'root', payload, ...extra,
    })!, nodeLabel)

    expect(line('PipelineStarted', { name: 'release_checklist' }, { node_id: 'release_checklist' })).toBe('Run started')
    expect(line('run_meta', { flow_name: 'release-checklist' })).toBe('Run details recorded')
    expect(line('StageStarted', {}, { node_id: 'run_tests' })).toBe('Run Test Suite started')
    expect(line('StageTransition', { decision: 'done', target: 'draft_notes' }, { node_id: 'run_tests' }))
      .toBe('Run Test Suite → Draft Notes on done')
    expect(line('StageTransition', { decision: 'ready_for_review', target: 'draft_notes' }, { node_id: 'run_tests' }))
      .toBe('Run Test Suite → Draft Notes on ready for review')
    // A fork enters its join on no decision.
    expect(line('StageTransition', { decision: '', target: 'draft_notes' }, { node_id: 'run_tests' }))
      .toBe('Run Test Suite → Draft Notes')
    expect(line('CheckpointSaved', { current_node: 'draft_notes' }, { node_id: 'draft_notes' })).toBe('Checkpoint saved at Draft Notes')
    expect(line('InterviewCompleted', { answer: 'ship' }, { node_id: 'draft_notes' })).toBe('Answer for Draft Notes: ship')
    // A child flow's nodes are its own; its parent node is this flow's.
    expect(line('StageStarted', {}, {
      node_id: 'run_tests', source_scope: 'child', source_parent_node_id: 'run_milestone', source_flow_name: 'child-flow',
    })).toBe('Child flow child-flow via Run Milestone: run_tests started')
    expect(line('log', { msg: 'hello' })).toBe('server log')
  })

  it('shows a gate answer once: as the gate took it, not also as delivered', () => {
    const entry = (sequence: number, rawType: string, payload: Record<string, unknown>) => toTimelineEvent({
      id: `journal-${sequence}`, sequence, emitted_at: '2026-07-08T10:00:00Z', kind: 'interview', raw_type: rawType,
      summary: rawType, severity: 'info', source_scope: 'root', node_id: 'approve', payload,
    })!
    const asked = entry(1, 'InterviewStarted', {})
    const delivered = entry(2, 'InterviewCompleted', { question_id: 'q-1', answer: 'approve' })
    const taken = entry(3, 'InterviewCompleted', { answer: 'Approve' })
    expect(journalEntriesShown([asked, delivered, taken])).toEqual([asked, taken])
    // Until the gate takes it, the delivered answer is the record.
    expect(journalEntriesShown([asked, delivered])).toEqual([asked, delivered])
  })

  it('says a checkpoint saved again at the exit once', () => {
    const entry = (sequence: number, rawType: string, node: string) => toTimelineEvent({
      id: `journal-${sequence}`, sequence, emitted_at: '2026-07-08T10:00:00Z', kind: 'checkpoint', raw_type: rawType,
      summary: rawType, severity: 'info', source_scope: 'root', node_id: node, payload: { current_node: node },
    })!
    const arrived = entry(1, 'CheckpointSaved', 'done')
    const ended = entry(2, 'CheckpointSaved', 'done')
    const finished = entry(3, 'PipelineCompleted', 'done')
    expect(journalEntriesShown([arrived, ended, finished])).toEqual([arrived, finished])
    // A loop that saves at the same node after other work says each save.
    const review = [entry(1, 'CheckpointSaved', 'review'), entry(2, 'StageStarted', 'review'), entry(3, 'CheckpointSaved', 'review')]
    expect(journalEntriesShown(review)).toEqual(review)
  })

  it('says a step that simply completed with the route it took next', () => {
    const entry = (sequence: number, rawType: string, node: string, payload: Record<string, unknown> = {}) => toTimelineEvent({
      id: `journal-${sequence}`, sequence, emitted_at: '2026-07-08T10:00:00Z', kind: 'stage', raw_type: rawType,
      summary: rawType, severity: 'info', source_scope: 'root', node_id: node, payload: { node_id: node, ...payload },
    })!
    const started = entry(1, 'StageStarted', 'tally')
    const completed = entry(2, 'StageCompleted', 'tally', { outcome: 'completed' })
    const routed = entry(3, 'StageTransition', 'tally', { target: 'climb', decision: 'climb' })
    const saved = toTimelineEvent({
      id: 'journal-4', sequence: 4, emitted_at: '2026-07-08T10:00:00Z', kind: 'checkpoint', raw_type: 'CheckpointSaved',
      summary: 'CheckpointSaved', severity: 'info', source_scope: 'root', node_id: 'climb', payload: { current_node: 'climb' },
    })!
    expect(journalEntriesShown([started, completed, saved, routed])).toEqual([started, saved, routed])
    // Another outcome is news, and so is a completion the route doesn't follow, such as the last step's.
    const partial = entry(2, 'StageCompleted', 'tally', { outcome: 'partial_success' })
    expect(journalEntriesShown([started, partial, routed])).toEqual([started, partial, routed])
    expect(journalEntriesShown([started, completed])).toEqual([started, completed])
  })

  it('tells a canceled run as canceled: its stopped branches, its end, and no status echoes', () => {
    const entry = (sequence: number, rawType: string, payload: Record<string, unknown>, severity = 'info') => toTimelineEvent({
      id: `journal-${sequence}`, sequence, emitted_at: '2026-07-08T10:00:00Z', kind: 'other', raw_type: rawType,
      summary: rawType === 'CancelRequested' ? 'Cancel requested' : `server ${rawType}`, severity, source_scope: 'root', node_id: payload.node_id ?? null,
      payload: { type: rawType, source_scope: 'root', ...payload },
    })!
    const labels: Record<string, string> = { lint: 'Lint check', unit: 'Unit check', checks: 'Checks' }
    const nodeLabel = (nodeId: string) => labels[nodeId] ?? nodeId
    const journal = [
      entry(1, 'runtime', { status: 'running' }),
      entry(2, 'PipelineStarted', { node_id: 'flow' }),
      entry(3, 'ParallelBranchStarted', { node_id: 'checks', branch: 'lint' }),
      entry(4, 'ParallelBranchNodeStarted', { node_id: 'checks', branch: 'lint', branch_node_id: 'lint' }),
      entry(5, 'ParallelBranchNodeStarted', { node_id: 'checks', branch: 'lint', branch_node_id: 'lint_fix' }),
      entry(6, 'CancelRequested', { status: 'cancel_requested' }, 'warning'),
      entry(7, 'runtime', { status: 'cancel_requested' }, 'warning'),
      entry(8, 'ParallelBranchNodeCompleted', { node_id: 'checks', branch: 'lint', branch_node_id: 'lint_fix', outcome: 'error' }),
      entry(9, 'ParallelBranchCompleted', { node_id: 'checks', branch: 'lint', success: false }),
      entry(10, 'ParallelCompleted', { node_id: 'checks', success_count: 0, failure_count: 2 }),
      entry(11, 'runtime', { status: 'canceled', last_error: 'aborted_by_user' }, 'warning'),
      entry(12, 'PipelineFailed', { error: 'aborted_by_user' }, 'warning'),
    ]
    let afterCancel = false
    const lines = journalEntriesShown(journal).map((event) => {
      afterCancel ||= event.type === 'CancelRequested'
      return journalEntrySummary(event, nodeLabel, afterCancel)
    })
    expect(lines).toEqual([
      'Run started',
      'Parallel branch Lint check started',
      'lint_fix started',
      'Cancel requested',
      'lint_fix stopped by the cancel',
      'Parallel branch Lint check stopped by the cancel',
      'Parallel fan-out stopped (0 of 2 branches finished)',
      'Run canceled',
    ])
    // Without its lifecycle event beside it, the status entry is the record and reads plainly.
    expect(journalEntriesShown([journal[10]]).map((event) => journalEntrySummary(event, nodeLabel))).toEqual(['Run canceled'])
    // A raw event of the end is a warning, not an error.
    expect(toTimelineEvent({ type: 'PipelineFailed', sequence: 1, emitted_at: '2026-07-08T10:00:00Z', error: 'aborted_by_user' })?.severity).toBe('warning')
  })

  it('tells a branch its fork stopped as stopped, not failed', () => {
    const entry = (sequence: number, rawType: string, payload: Record<string, unknown>) => toTimelineEvent({
      id: `journal-${sequence}`, sequence, emitted_at: '2026-07-08T10:00:00Z', kind: 'other', raw_type: rawType,
      summary: `server ${rawType}`, severity: 'info', source_scope: 'root', node_id: 'checks',
      payload: { type: rawType, source_scope: 'root', node_id: 'checks', ...payload },
    })!
    const lines = [
      entry(1, 'ParallelBranchCompleted', { branch: 'typecheck', success: false, status: 'error' }),
      entry(2, 'ParallelBranchCompleted', { branch: 'slow', success: false, status: 'canceled' }),
      entry(3, 'ParallelCompleted', { success_count: 0, failure_count: 2, canceled_count: 1 }),
    ].map((event) => journalEntrySummary(event, (nodeId) => nodeId))
    expect(lines).toEqual([
      'Parallel branch typecheck completed (failed)',
      'Parallel branch slow stopped',
      'Parallel fan-out stopped (0 of 2 branches finished: 1 failed, 1 stopped)',
    ])
  })

  it('derives pending interview gates from live events and fallback question snapshots', () => {
    const timeoutEvent = toTimelineEvent({
      type: 'human_gate',
      sequence: 1,
      emitted_at: '2026-04-06T12:01:00Z',
      node_id: 'review',
      prompt: 'Need approval?',
      question_type: 'YES_NO',
    })

    const pendingGates = buildPendingInterviewGates(
      [timeoutEvent].filter(Boolean),
      [
        {
          questionId: 'q-2',
          nodeId: 'review',
          prompt: 'Provide extra detail',
          questionType: 'FREEFORM',
          options: [],
          askedAt: '2026-04-06T12:02:00Z',
        },
      ],
    )

    expect(pendingGates).toHaveLength(2)
    expect(pendingGates[0]).toMatchObject({
      nodeId: 'review',
      questionType: 'YES_NO',
      options: [
        { label: 'Yes', value: 'YES', key: 'Y', description: null },
        { label: 'No', value: 'NO', key: 'N', description: null },
      ],
    })
    // A question known only from the questions API keeps the time it was asked.
    expect(pendingGates[1]).toMatchObject({
      questionId: 'q-2',
      prompt: 'Provide extra detail',
      receivedAt: '2026-04-06T12:02:00Z',
    })

    const grouped = buildGroupedPendingInterviewGates(pendingGates)
    expect(grouped).toHaveLength(1)
    expect(grouped[0]?.heading).toBe('review')
  })

  it('preserves child source labels', () => {
    const childEvent = toTimelineEvent({
      type: 'StageStarted',
      node_id: 'plan_current',
      index: 1,
      sequence: 12,
      emitted_at: '2026-04-06T12:00:00Z',
      source_scope: 'child',
      source_parent_node_id: 'run_milestone',
      source_flow_name: 'implement-milestone.dot',
    })

    expect(childEvent).toMatchObject({
      sourceScope: 'child',
      sourceParentNodeId: 'run_milestone',
      sourceFlowName: 'implement-milestone.dot',
      summary: 'Child flow implement-milestone.dot via run_milestone: plan_current started',
      receivedAt: '2026-04-06T12:00:00Z',
    })
  })

  it('groups child pending interview gates under child-aware headings', () => {
    const childGateEvent = toTimelineEvent({
      type: 'human_gate',
      node_id: 'review_gate',
      prompt: 'Approve the child milestone?',
      question_type: 'YES_NO',
      sequence: 7,
      emitted_at: '2026-04-06T12:05:00Z',
      source_scope: 'child',
      source_parent_node_id: 'run_milestone',
      source_flow_name: 'implement-milestone.dot',
    })

    const pendingGates = buildPendingInterviewGates([childGateEvent].filter(Boolean), [])
    const grouped = buildGroupedPendingInterviewGates(pendingGates)

    expect(grouped).toHaveLength(1)
    expect(grouped[0]?.heading).toBe('review_gate · Child flow implement-milestone.dot via run_milestone')
  })

  it('rejects timeline events that omit stable server sequence or timestamp', () => {
    expect(toTimelineEvent({
      type: 'StageStarted',
      emitted_at: '2026-04-06T12:04:00Z',
      node_id: 'plan',
      index: 1,
    })).toBeNull()

    expect(toTimelineEvent({
      type: 'StageStarted',
      sequence: 4,
      node_id: 'plan',
      index: 1,
    })).toBeNull()
  })

})

describe('humanizeTimelineType', () => {
  it('maps known runtime types to human labels', () => {
    expect(humanizeTimelineType('PipelineCompleted')).toBe('Run completed')
    expect(humanizeTimelineType('human_gate')).toBe('Waiting for input')
    expect(humanizeTimelineType('LLMContent')).toBe('Assistant output')
  })

  it('falls back to spaced sentence case for unknown types', () => {
    expect(humanizeTimelineType('SomeNewEvent')).toBe('Some new event')
    expect(humanizeTimelineType('another_raw_kind')).toBe('Another raw kind')
  })
})

it('keeps clarification identities distinct, routes child answers, and hides orphaned prompts', () => {
  const question = (id: string) => toTimelineEvent({
    type: 'human_gate', sequence: 1, emitted_at: '2026-09-08T00:00:00Z', question_id: id, node_id: 'work',
    run_id: 'child', origin: 'agent_clarification', prompt: 'Which audience?',
    question_type: 'FREEFORM', options: [{ label: 'Developers', value: 'Developers' }],
  })!
  const snapshot = (id: string) => ({
    questionId: id, runId: 'child', origin: 'agent_clarification', nodeId: 'work',
    prompt: 'Which audience?', questionType: 'FREEFORM' as const,
    options: [{ label: 'Developers', value: 'Developers', key: null, description: null }],
  })
  const events = [question('first'), question('second')]
  const pending = buildPendingInterviewGates(events, [snapshot('first'), snapshot('second')])
  expect(pending.map((q) => q.questionId)).toEqual(['first', 'second'])
  expect(pending.every((q) => q.runId === 'child')).toBe(true)
  expect(buildPendingInterviewGates(events, [snapshot('second')]).map((q) => q.questionId)).toEqual(['second'])
  expect(buildPendingInterviewGates(events, [])).toEqual([])
})

it('carries a harness form from its gate event or snapshot to the one pending gate', () => {
  const form = { request_id: 'acp-form-1', harness: 'acp:x', harness_label: 'X Agent', message: 'Pick a size', schema: { type: 'object', properties: {} } }
  const event = toTimelineEvent({
    type: 'human_gate', sequence: 1, emitted_at: '2026-09-08T00:00:00Z', question_id: 'f', node_id: 'work',
    run_id: 'run', origin: 'agent_clarification', prompt: 'Pick a size', question_type: 'FORM', form,
  })!
  const snapshot = { questionId: 'f', runId: 'run', origin: 'agent_clarification', nodeId: 'work', prompt: 'Pick a size', questionType: null, options: [], askedAt: '' }
  const expected = { requestId: 'acp-form-1', harness: 'acp:x', harnessLabel: 'X Agent', message: 'Pick a size', schema: form.schema }
  expect(buildPendingInterviewGates([event], [snapshot])).toMatchObject([{ questionId: 'f', questionType: null, options: [], form: expected }])
  expect(buildPendingInterviewGates([], [{ ...snapshot, form: expected }])).toMatchObject([{ questionId: 'f', form: expected }])
})
