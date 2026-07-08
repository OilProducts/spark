import {
  buildGroupedPendingInterviewGates,
  buildPendingInterviewGates,
  filterTimelineEvents,
  toTimelineEvent,
} from '@/features/runs/model/timelineModel'

describe('timelineModel', () => {
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
      summary: 'Pipeline completed at done (failure: Goal gate not satisfied)',
    })
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

  it('filters timeline events by severity and node/stage text', () => {
    const events = [
      toTimelineEvent({
        type: 'StageStarted',
        sequence: 1,
        emitted_at: '2026-04-06T12:02:00Z',
        node_id: 'plan',
        index: 1,
      }),
      toTimelineEvent({
        type: 'StageFailed',
        sequence: 2,
        emitted_at: '2026-04-06T12:03:00Z',
        node_id: 'apply',
        index: 2,
        error: 'boom',
      }),
    ].filter(Boolean)

    const filtered = filterTimelineEvents(events, {
      timelineTypeFilter: 'all',
      timelineCategoryFilter: 'all',
      timelineSeverityFilter: 'error',
      timelineNodeStageFilter: 'apply',
    })

    expect(filtered).toHaveLength(1)
    expect(filtered[0]?.nodeId).toBe('apply')
  })

  it('preserves child source labels and allows filtering by child metadata', () => {
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
      summary: 'Child flow implement-milestone.dot via run_milestone: Stage plan_current started',
      receivedAt: '2026-04-06T12:00:00Z',
    })

    const filtered = filterTimelineEvents([childEvent].filter(Boolean), {
      timelineTypeFilter: 'all',
      timelineCategoryFilter: 'all',
      timelineSeverityFilter: 'all',
      timelineNodeStageFilter: 'run_milestone',
    })

    expect(filtered).toHaveLength(1)
    expect(filtered[0]?.id).toBe('event-12')
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
