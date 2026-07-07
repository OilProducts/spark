import {
  buildGroupedPendingInterviewGates,
  buildPendingInterviewGates,
  buildRunProgressProjection,
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

  const transcriptSegment = (overrides: Record<string, unknown>) => ({
    id: 'segment-assistant-run-node-draft',
    turn_id: 'run-node-draft',
    order: 1,
    kind: 'assistant_message',
    role: 'assistant',
    status: 'complete',
    timestamp: '2026-04-06T12:00:00Z',
    updated_at: '2026-04-06T12:00:02Z',
    content: 'Draft output',
    ...overrides,
  })

  it('builds progress entries from coalesced transcript segments', () => {
    const projection = buildRunProgressProjection([
      transcriptSegment({
        content: '## Draft\n\nUse **markdown**.',
        order: 3,
      }),
    ] as never, null)

    expect(projection.recentEntries).toHaveLength(1)
    expect(projection.recentEntries[0]).toMatchObject({
      nodeId: 'draft',
      channel: 'assistant',
      status: 'complete',
      content: '## Draft\n\nUse **markdown**.',
      latestSequence: 3,
    })
  })

  it('selects the newest current-node transcript stream as active progress', () => {
    const projection = buildRunProgressProjection([
      transcriptSegment({ order: 1 }),
      transcriptSegment({
        id: 'segment-assistant-run-node-validate',
        turn_id: 'run-node-validate',
        order: 2,
        content: 'Older validate output',
      }),
      transcriptSegment({
        id: 'segment-reasoning-run-node-validate-r-0',
        turn_id: 'run-node-validate',
        kind: 'reasoning',
        order: 3,
        content: 'Newest validate output',
      }),
    ] as never, 'validate')

    expect(projection.activeEntry).toMatchObject({
      nodeId: 'validate',
      channel: 'reasoning',
      content: 'Newest validate output',
      latestSequence: 3,
    })
    expect(projection.recentEntries.map((entry) => entry.id)).not.toContain(projection.activeEntry?.id)
    expect(projection.nodeOptions).toEqual(['validate', 'draft'])
  })

  it('falls back to recent progress entries when the current node has no LLM content', () => {
    const projection = buildRunProgressProjection([
      transcriptSegment({}),
    ] as never, 'validate')

    expect(projection.activeEntry).toBeNull()
    expect(projection.recentEntries).toHaveLength(1)
    expect(projection.recentEntries[0]).toMatchObject({ nodeId: 'draft' })
  })

  it('supports concrete node filtering from the progress projection', () => {
    const projection = buildRunProgressProjection([
      transcriptSegment({}),
      transcriptSegment({
        id: 'segment-assistant-run-node-validate',
        turn_id: 'run-node-validate',
        order: 2,
        content: 'Validate output',
      }),
    ] as never, 'validate')

    const visibleEntries = [
      projection.activeEntry,
      ...projection.recentEntries,
    ].filter((entry) => entry?.nodeId === 'draft')

    expect(visibleEntries).toHaveLength(1)
    expect(visibleEntries[0]).toMatchObject({
      nodeId: 'draft',
      content: 'Draft output',
    })
  })

  it('does not build progress rows from boundaries, tool calls, or empty segments', () => {
    const projection = buildRunProgressProjection([
      transcriptSegment({
        id: 'boundary-root-root--implement-0-1',
        kind: 'boundary',
        status: 'completed',
        content: 'Stage implement completed',
        boundary: { node_id: 'implement', source_scope: 'root', summary: 'Stage implement completed' },
      }),
      transcriptSegment({
        id: 'segment-tool-run-node-implement-call-1',
        kind: 'tool_call',
        role: 'system',
        content: 'Run ls',
        tool_call: { id: 'call-1', kind: 'command_execution', status: 'completed', title: 'Run ls', file_paths: [] },
      }),
      transcriptSegment({ content: '   ' }),
    ] as never, null)

    expect(projection.activeEntry).toBeNull()
    expect(projection.recentEntries).toHaveLength(0)
    expect(projection.nodeOptions).toEqual([])
  })
})
