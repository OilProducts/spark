import {
  buildGroupedPendingInterviewGates,
  buildPendingInterviewGates,
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
    expect(line('CheckpointSaved', { current_node: 'draft_notes' }, { node_id: 'draft_notes' })).toBe('Checkpoint saved at Draft Notes')
    expect(line('InterviewCompleted', { answer: 'ship' }, { node_id: 'draft_notes' })).toBe('Answer for Draft Notes: ship')
    // A child flow's nodes are its own; its parent node is this flow's.
    expect(line('StageStarted', {}, {
      node_id: 'run_tests', source_scope: 'child', source_parent_node_id: 'run_milestone', source_flow_name: 'child-flow',
    })).toBe('Child flow child-flow via Run Milestone: run_tests started')
    expect(line('log', { msg: 'hello' })).toBe('server log')
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
    expect(pendingGates[1]).toMatchObject({
      questionId: 'q-2',
      prompt: 'Provide extra detail',
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
