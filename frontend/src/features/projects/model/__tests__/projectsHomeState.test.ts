import {
  applyConversationSnapshotToCache,
  applyConversationStreamEventToCache,
  EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
  getConversationTimelineEntries,
  hydrateConversationRecordFromSnapshot,
  removeProjectFromCache,
} from '@/features/projects/model/projectsHomeState'
import type {
  ConversationSegmentResponse,
  ConversationSnapshotResponse,
  ConversationTurnResponse,
  FlowRunRequestResponse,
  ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'

const buildTurn = (overrides: Partial<ConversationTurnResponse> = {}): ConversationTurnResponse => ({
  id: 'turn-assistant',
  role: 'assistant',
  content: 'I prepared the run request for review.',
  timestamp: '2026-03-06T15:00:10Z',
  kind: 'message',
  status: 'complete',
  artifact_id: null,
  ...overrides,
})

const buildSegment = (overrides: Partial<ConversationSegmentResponse> = {}): ConversationSegmentResponse => ({
  id: 'segment-assistant',
  turn_id: 'turn-assistant',
  order: 1,
  kind: 'assistant_message',
  role: 'assistant',
  status: 'complete',
  timestamp: '2026-03-06T15:00:10Z',
  updated_at: '2026-03-06T15:00:10Z',
  completed_at: '2026-03-06T15:00:10Z',
  content: 'I prepared the run request for review.',
  artifact_id: null,
  error: null,
  tool_call: null,
  source: null,
  ...overrides,
})

const buildFlowRunRequest = (overrides: Partial<FlowRunRequestResponse> = {}): FlowRunRequestResponse => ({
  id: 'request-1',
  created_at: '2026-03-06T15:00:20Z',
  updated_at: '2026-03-06T15:00:20Z',
  flow_name: 'implementation.dot',
  summary: 'Launch the implementation flow.',
  project_path: '/tmp/project-contract-behavior',
  conversation_id: 'conversation-1',
  source_turn_id: 'turn-assistant',
  source_segment_id: 'segment-request-1',
  status: 'pending',
  goal: 'Implement the approved request.',
  launch_context: null,
  model: null,
  llm_provider: null,
  llm_profile: null,
  reasoning_effort: null,
  run_id: null,
  launch_error: null,
  review_message: null,
  ...overrides,
})

const buildProposedPlan = (overrides: Partial<ProposedPlanArtifactResponse> = {}): ProposedPlanArtifactResponse => ({
  id: 'plan-1',
  created_at: '2026-03-06T15:00:30Z',
  updated_at: '2026-03-06T15:00:30Z',
  title: 'Implementation plan',
  content: 'Do the work incrementally.',
  project_path: '/tmp/project-contract-behavior',
  conversation_id: 'conversation-1',
  source_turn_id: 'turn-assistant',
  source_segment_id: 'segment-plan-1',
  status: 'pending_review',
  review_note: null,
  written_change_request_path: null,
  flow_launch_id: null,
  run_id: null,
  launch_error: null,
  ...overrides,
})

const buildSnapshot = (
  overrides: Partial<ConversationSnapshotResponse> = {},
): ConversationSnapshotResponse => ({
  schema_version: 5,
  revision: 0,
  conversation_id: 'conversation-1',
  conversation_handle: '',
  project_path: '/tmp/project-contract-behavior',
  chat_mode: 'chat',
  title: 'Contract behavior',
  created_at: '2026-03-06T15:00:00Z',
  updated_at: '2026-03-06T15:01:00Z',
  turns: [
    {
      id: 'turn-user',
      role: 'user',
      content: 'Draft a plan.',
      timestamp: '2026-03-06T15:00:00Z',
      kind: 'message',
      status: 'complete',
      artifact_id: null,
    },
    buildTurn(),
  ],
  segments: [
    buildSegment(),
  ],
  event_log: [
    {
      message: 'Flow run request request-1 approved for launch.',
      timestamp: '2026-03-06T15:01:00Z',
    },
  ],
  flow_run_requests: [],
  flow_launches: [],
  proposed_plans: [],
  ...overrides,
})

describe('applyConversationSnapshotToCache', () => {
  it('hydrates snapshots into normalized conversation records with ordered timeline and artifact maps', () => {
    const snapshot = buildSnapshot({
      model: 'gpt-5.5',
      reasoning_effort: 'high',
      segments: [
        buildSegment({
          id: 'segment-request-1',
          order: 2,
          kind: 'flow_run_request',
          role: 'system',
          content: 'Review request',
          artifact_id: 'request-1',
        }),
        buildSegment({
          id: 'segment-assistant',
          order: 1,
        }),
        buildSegment({
          id: 'segment-plan-1',
          order: 3,
          kind: 'plan',
          content: 'Do the work incrementally.',
          artifact_id: 'plan-1',
        }),
      ],
      flow_run_requests: [buildFlowRunRequest()],
      proposed_plans: [buildProposedPlan()],
    })

    const record = hydrateConversationRecordFromSnapshot(snapshot)

    expect(record.orderedTurnIds).toEqual(['turn-user', 'turn-assistant'])
    expect(record.orderedSegmentIdsByTurnId['turn-assistant']).toEqual([
      'segment-assistant',
      'segment-request-1',
      'segment-plan-1',
    ])
    expect(record.flowRunRequestIds).toEqual(['request-1'])
    expect(record.flowRunRequestsById['request-1']?.status).toBe('pending')
    expect(record.proposedPlanIds).toEqual(['plan-1'])
    expect(record.proposedPlansById['plan-1']?.title).toBe('Implementation plan')
    expect(getConversationTimelineEntries(record).map((entry) => entry.id)).toEqual([
      'turn-user',
      'segment-assistant',
      'segment-request-1',
      'segment-plan-1',
    ])
  })

  it('accepts same-timestamp snapshots when revision advances', () => {
    const initialSnapshot = buildSnapshot()
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const updatedSnapshot = buildSnapshot({
      revision: initialSnapshot.revision + 1,
      event_log: [
        ...initialSnapshot.event_log,
        {
          message: 'Flow launch run-1 completed successfully.',
          timestamp: '2026-03-06T15:02:00Z',
        },
      ],
    })

    const result = applyConversationSnapshotToCache(
      cacheWithInitialSnapshot,
      updatedSnapshot.project_path,
      updatedSnapshot,
    )

    expect(result.applied).toBe(true)
    expect(result.cache.conversationsById[updatedSnapshot.conversation_id]?.event_log).toHaveLength(2)
  })

  it('rejects stale snapshots instead of replacing a fresher normalized record', () => {
    const freshSnapshot = buildSnapshot({
      revision: 2,
      updated_at: '2026-03-06T15:03:00Z',
      turns: [
        buildSnapshot().turns[0],
        buildTurn({ content: 'Fresher assistant content.' }),
      ],
      segments: [
        buildSegment({ content: 'Fresher assistant content.' }),
      ],
    })
    const cacheWithFreshSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      freshSnapshot.project_path,
      freshSnapshot,
    ).cache

    const staleSnapshot = buildSnapshot({
      revision: 1,
      updated_at: '2026-03-06T15:02:00Z',
      turns: [
        buildSnapshot().turns[0],
        buildTurn({ content: 'Stale assistant content.' }),
      ],
      segments: [
        buildSegment({ content: 'Stale assistant content.' }),
      ],
    })

    const result = applyConversationSnapshotToCache(
      cacheWithFreshSnapshot,
      staleSnapshot.project_path,
      staleSnapshot,
    )

    expect(result.applied).toBe(false)
    expect(result.cache.conversationsById[staleSnapshot.conversation_id]?.turnsById['turn-assistant']?.content)
      .toBe('Fresher assistant content.')
  })

  it('rejects equal-revision snapshots even when content changes', () => {
    const initialSnapshot = buildSnapshot({ revision: 4 })
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const equalRevisionSnapshot = buildSnapshot({
      revision: 4,
      updated_at: '2026-03-06T15:03:00Z',
      turns: [
        buildSnapshot().turns[0],
        buildTurn({ content: 'Equal revision should not replace cached content.' }),
      ],
    })

    const result = applyConversationSnapshotToCache(
      cacheWithInitialSnapshot,
      equalRevisionSnapshot.project_path,
      equalRevisionSnapshot,
    )

    expect(result.applied).toBe(false)
    expect(result.cache.conversationsById[equalRevisionSnapshot.conversation_id]?.turnsById['turn-assistant']?.content)
      .toBe('I prepared the run request for review.')
  })

  it('updates cached chat_mode when a mode_change turn-upsert arrives', () => {
    const initialSnapshot = buildSnapshot()
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const result = applyConversationStreamEventToCache(
      cacheWithInitialSnapshot,
      initialSnapshot.project_path,
      {
        type: 'turn_upsert',
        revision: initialSnapshot.revision,
        conversation_id: initialSnapshot.conversation_id,
        project_path: initialSnapshot.project_path,
        title: initialSnapshot.title,
        updated_at: '2026-03-06T15:01:30Z',
        turn: {
          id: 'turn-mode-1',
          role: 'system',
          kind: 'mode_change',
          status: 'complete',
          content: 'plan',
          timestamp: '2026-03-06T15:01:30Z',
          artifact_id: null,
        },
      },
    )

    expect(result.record.chat_mode).toBe('plan')
    expect(result.cache.conversationsById[initialSnapshot.conversation_id]?.chat_mode).toBe('plan')
  })

  it('appends new turn_upsert turns and updates existing turns in place', () => {
    const initialSnapshot = buildSnapshot()
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const appendResult = applyConversationStreamEventToCache(
      cacheWithInitialSnapshot,
      initialSnapshot.project_path,
      {
        type: 'turn_upsert',
        revision: 1,
        conversation_id: initialSnapshot.conversation_id,
        project_path: initialSnapshot.project_path,
        title: 'Updated thread',
        updated_at: '2026-03-06T15:01:30Z',
        turn: buildTurn({
          id: 'turn-assistant-2',
          content: '',
          status: 'streaming',
          timestamp: '2026-03-06T15:01:30Z',
        }),
      },
    )

    expect(appendResult.record.orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'turn-assistant-2'])
    expect(getConversationTimelineEntries(appendResult.record)).toContainEqual(expect.objectContaining({
      id: 'turn-assistant-2:thinking:placeholder',
      content: '',
      presentation: 'thinking',
    }))

    const updateResult = applyConversationStreamEventToCache(
      appendResult.cache,
      initialSnapshot.project_path,
      {
        type: 'turn_upsert',
        revision: 2,
        conversation_id: initialSnapshot.conversation_id,
        project_path: initialSnapshot.project_path,
        title: 'Updated thread',
        updated_at: '2026-03-06T15:02:00Z',
        turn: buildTurn({
          id: 'turn-assistant-2',
          content: 'The appended turn is now complete.',
          status: 'complete',
          timestamp: '2026-03-06T15:01:30Z',
        }),
      },
    )

    expect(updateResult.record.orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'turn-assistant-2'])
    expect(updateResult.record.turnsById['turn-assistant-2']?.content).toBe('The appended turn is now complete.')
    expect(getConversationTimelineEntries(updateResult.record)).toContainEqual(expect.objectContaining({
      id: 'turn-assistant-2:default:placeholder',
      content: 'The appended turn is now complete.',
      presentation: 'default',
    }))
  })

  it('patches streaming segment content on the normalized turn timeline', () => {
    const initialSnapshot = buildSnapshot({
      segments: [
        {
          ...buildSnapshot().segments[0],
          id: 'assistant-stream',
          kind: 'assistant_message',
          status: 'running',
          content: 'Hel',
          order: 1,
          tool_call: null,
        },
      ],
    })
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const result = applyConversationStreamEventToCache(
      cacheWithInitialSnapshot,
      initialSnapshot.project_path,
      {
        type: 'segment_upsert',
        revision: 1,
        conversation_id: initialSnapshot.conversation_id,
        project_path: initialSnapshot.project_path,
        title: initialSnapshot.title,
        updated_at: '2026-03-06T15:01:45Z',
        segment: {
          ...initialSnapshot.segments[0],
          content: 'Hello from streaming.',
        },
      },
    )

    expect(getConversationTimelineEntries(result.record)).toContainEqual(expect.objectContaining({
      id: 'assistant-stream',
      kind: 'message',
      content: 'Hello from streaming.',
    }))
    expect(result.record.orderedTurnIds).toEqual(['turn-user', 'turn-assistant'])
  })

  it('reports stream events for unknown conversations as missing records', () => {
    const result = applyConversationStreamEventToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      '/tmp/project-contract-behavior',
      {
        type: 'turn_upsert',
        revision: 1,
        conversation_id: 'conversation-missing',
        project_path: '/tmp/project-contract-behavior',
        title: 'Missing conversation',
        updated_at: '2026-03-06T15:01:30Z',
        turn: buildTurn({
          id: 'turn-missing',
          content: 'This should wait for a real snapshot.',
          timestamp: '2026-03-06T15:01:30Z',
        }),
      },
    )

    expect(result.status).toBe('missing_record')
    expect(result.cache).toBe(EMPTY_PROJECT_CONVERSATION_CACHE_STATE)
    expect(result.cache.conversationsById['conversation-missing']).toBeUndefined()
  })

  it('ignores stream events below the cached revision', () => {
    const initialSnapshot = buildSnapshot({
      revision: 5,
      turns: [
        buildSnapshot().turns[0],
        buildTurn({ content: 'Cached assistant content.' }),
      ],
    })
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const result = applyConversationStreamEventToCache(
      cacheWithInitialSnapshot,
      initialSnapshot.project_path,
      {
        type: 'turn_upsert',
        revision: 4,
        conversation_id: initialSnapshot.conversation_id,
        project_path: initialSnapshot.project_path,
        title: initialSnapshot.title,
        updated_at: '2026-03-06T15:02:00Z',
        turn: buildTurn({ content: 'Outdated stream content.' }),
      },
    )

    expect(result.record.turnsById['turn-assistant']?.content).toBe('Cached assistant content.')
    expect(result.cache).toBe(cacheWithInitialSnapshot)
  })

  it('hydrates refreshed artifact snapshots into normalized artifact records and timeline rows', () => {
    const initialSnapshot = buildSnapshot({
      updated_at: '2026-03-06T15:01:00Z',
      segments: [
        buildSegment({
          id: 'segment-request-1',
          kind: 'flow_run_request',
          role: 'system',
          content: 'Review request',
          artifact_id: 'request-1',
        }),
      ],
      flow_run_requests: [
        buildFlowRunRequest({
          status: 'pending',
          updated_at: '2026-03-06T15:01:00Z',
        }),
      ],
    })
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const refreshedSnapshot = buildSnapshot({
      revision: initialSnapshot.revision + 1,
      updated_at: '2026-03-06T15:02:00Z',
      segments: initialSnapshot.segments,
      flow_run_requests: [
        buildFlowRunRequest({
          status: 'approved',
          run_id: 'run-1',
          updated_at: '2026-03-06T15:02:00Z',
        }),
      ],
      event_log: [
        ...initialSnapshot.event_log,
        {
          message: 'Flow run request request-1 approved for launch.',
          timestamp: '2026-03-06T15:02:00Z',
        },
      ],
    })

    const result = applyConversationSnapshotToCache(
      cacheWithInitialSnapshot,
      refreshedSnapshot.project_path,
      refreshedSnapshot,
    )

    expect(result.applied).toBe(true)
    expect(result.record?.flowRunRequestsById['request-1']?.status).toBe('approved')
    expect(result.record?.flowRunRequestsById['request-1']?.run_id).toBe('run-1')
    expect(getConversationTimelineEntries(result.record ?? null)).toContainEqual(expect.objectContaining({
      id: 'segment-request-1',
      kind: 'flow_run_request',
      artifactId: 'request-1',
    }))
  })

  it('removes normalized conversations and summaries when a project cache is deleted', () => {
    const projectSnapshot = buildSnapshot()
    const otherSnapshot = buildSnapshot({
      conversation_id: 'conversation-other',
      project_path: '/tmp/other-project',
      title: 'Other project',
      turns: [
        buildTurn({
          id: 'turn-other',
          content: 'Other project message.',
        }),
      ],
      segments: [
        buildSegment({
          id: 'segment-other',
          turn_id: 'turn-other',
          content: 'Other project message.',
        }),
      ],
    })
    const cacheWithProject = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      projectSnapshot.project_path,
      projectSnapshot,
    ).cache
    const cacheWithBothProjects = applyConversationSnapshotToCache(
      cacheWithProject,
      otherSnapshot.project_path,
      otherSnapshot,
    ).cache

    const result = removeProjectFromCache(cacheWithBothProjects, projectSnapshot.project_path)

    expect(result.conversationsById[projectSnapshot.conversation_id]).toBeUndefined()
    expect(result.summariesByProjectPath[projectSnapshot.project_path]).toBeUndefined()
    expect(result.conversationsById[otherSnapshot.conversation_id]?.project_path).toBe(otherSnapshot.project_path)
    expect(result.summariesByProjectPath[otherSnapshot.project_path]).toHaveLength(1)
  })
})
