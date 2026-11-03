import {
  applyConversationBatchToCache,
  applyConversationPage,
  applyConversationPageToCache,
  applySummaryBatchToCache,
  applySummarySnapshotToCache,
  applyTransientConversationEventToCache,
  EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
  getConversationTimelineEntries,
  removeProjectFromCache,
  resetConversationInCache,
  type ProjectConversationCacheState,
} from '@/features/projects/model/projectsHomeState'
import { lastChat, sourceOf, tally } from '@/features/overview/model/overviewModel'
import {
  parseConversationBatchResponse,
  parseConversationPageResponse,
  parseConversationSummarySnapshotResponse,
  type ConversationSegmentResponse,
  type ConversationStreamDeltaEventResponse,
  type ConversationTurnResponse,
  type FlowLaunchResponse,
  type FlowRunRequestResponse,
  type ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'
import {
  artifactRow,
  conversationBatch,
  conversationPage,
  conversationRow,
  segmentRow,
  summarySnapshot,
  tombstoneRow,
  turnRow,
} from '@/test/conversationWire'

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

const buildFlowLaunch = (overrides: Partial<FlowLaunchResponse> = {}): FlowLaunchResponse => ({
  id: 'launch-1',
  created_at: '2026-03-06T15:00:25Z',
  updated_at: '2026-03-06T15:00:25Z',
  flow_name: 'implementation.dot',
  summary: 'Launch the implementation flow.',
  project_path: '/tmp/project-contract-behavior',
  conversation_id: 'conversation-1',
  source_turn_id: 'turn-assistant',
  source_segment_id: 'segment-launch-1',
  status: 'pending',
  goal: 'Implement the approved request.',
  launch_context: null,
  model: null,
  llm_provider: null,
  llm_profile: null,
  reasoning_effort: null,
  run_id: null,
  launch_error: null,
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

type Snapshot = Record<string, unknown> & {
  revision: number
  conversation_id: string
  project_path: string
  title: string
  turns: ConversationTurnResponse[]
  segments: ConversationSegmentResponse[]
  flow_run_requests: FlowRunRequestResponse[]
  flow_launches: FlowLaunchResponse[]
  proposed_plans: ProposedPlanArtifactResponse[]
}

const buildSnapshot = (overrides: Partial<Snapshot> = {}): Snapshot => ({
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
  flow_run_requests: [],
  flow_launches: [],
  proposed_plans: [],
  ...overrides,
})

const PROJECT = '/tmp/project-contract-behavior'
const ID = 'conversation-1'
type Row = Record<string, unknown>
type PageOptions = Parameters<typeof conversationPage>[1]
type BatchOptions = Parameters<typeof conversationBatch>[1]

const page = (snapshot: Snapshot, options?: PageOptions) => parseConversationPageResponse(conversationPage(snapshot, options))
const open = (snapshot: Snapshot, cache: ProjectConversationCacheState = EMPTY_PROJECT_CONVERSATION_CACHE_STATE, options?: PageOptions) => (
  applyConversationPageToCache(cache, page(snapshot, options), 'window')
)
const older = (cache: ProjectConversationCacheState, snapshot: Snapshot, options?: PageOptions) => (
  applyConversationPageToCache(cache, page(snapshot, options), 'older')
)
const live = (cache: ProjectConversationCacheState, rows: Row[], options?: BatchOptions) => (
  applyConversationBatchToCache(cache, ID, parseConversationBatchResponse(conversationBatch(rows, options)), 'live')
)
const ack = (cache: ProjectConversationCacheState, rows: Row[], options?: BatchOptions) => (
  applyConversationBatchToCache(cache, ID, parseConversationBatchResponse(conversationBatch(rows, { from_revision: '0', ...options }), undefined, 'mutation'), 'mutation')
)
const recordOf = (cache: ProjectConversationCacheState) => cache.conversationsById[ID]!
const turn = (value: ConversationTurnResponse, revision: number, position?: number) => turnRow(ID, value, String(revision), position)
const segment = (value: ConversationSegmentResponse, revision: number) => segmentRow(ID, value, String(revision))
const conversation = (revision: number, overrides: Partial<Snapshot> = {}) => conversationRow(buildSnapshot(overrides), String(revision))
const timelineIds = (cache: ProjectConversationCacheState) => getConversationTimelineEntries(recordOf(cache)).map((entry) => entry.id)

describe('applyConversationPageToCache', () => {
  it('hydrates a page into a normalized record with ordered timeline and artifact maps', () => {
    const cache = open(buildSnapshot({
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
    }))
    const record = recordOf(cache)

    expect(record.model).toBe('gpt-5.5')
    expect(record.reasoning_effort).toBe('high')
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
    expect(record.timelineEntryIds).toEqual([
      'turn-user',
      'segment-assistant',
      'segment-request-1',
      'segment-plan-1',
    ])
    expect(record.timelineEntryIdsByTurnId).toEqual({
      'turn-user': ['turn-user'],
      'turn-assistant': ['segment-assistant', 'segment-request-1', 'segment-plan-1'],
    })
    expect(record.timelineEntriesById['segment-plan-1']).toMatchObject({
      kind: 'plan',
      artifactId: 'plan-1',
    })
    expect(getConversationTimelineEntries(record).map((entry) => entry.id)).toEqual(record.timelineEntryIds)
  })

  it('links a plan segment to the proposed plan that names it as its source', () => {
    const record = recordOf(open(buildSnapshot({
      segments: [buildSegment({ id: 'segment-plan-1', kind: 'plan', content: 'Do the work incrementally.', artifact_id: null })],
      proposed_plans: [buildProposedPlan()],
    })))

    expect(record.segmentsById['segment-plan-1']?.artifact_id).toBe('plan-1')
    expect(record.timelineEntriesById['segment-plan-1']).toMatchObject({ kind: 'plan', artifactId: 'plan-1' })
  })

  it('orders turns by position and segments by order, both compared as integers beyond float precision', () => {
    const cache = open(buildSnapshot({
      turns: [
        buildTurn({ id: 'turn-late', role: 'user', content: 'Late.', position: '90071992547409930' } as Partial<ConversationTurnResponse>),
        buildTurn({ id: 'turn-early', role: 'user', content: 'Early.', position: '90071992547409920' } as Partial<ConversationTurnResponse>),
      ],
      segments: [],
    }))
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-early', 'turn-late'])
  })

  it('hydrates many same-turn segments using stable bulk ordering', () => {
    const segmentCount = 160
    const turnId = 'turn-assistant-large'
    const segments: ConversationSegmentResponse[] = Array.from({ length: segmentCount }, (_, offset) => {
      const index = segmentCount - offset - 1
      return buildSegment({
        id: `${turnId}-message-${index.toString().padStart(3, '0')}`,
        turn_id: turnId,
        order: index,
        timestamp: `2026-03-06T15:00:${(index % 60).toString().padStart(2, '0')}Z`,
        content: `Message ${index}`,
      })
    })
    const expectedSegmentIds = Array.from(
      { length: segmentCount },
      (_, index) => `${turnId}-message-${index.toString().padStart(3, '0')}`,
    )

    const record = recordOf(open(buildSnapshot({
      turns: [buildTurn({ id: turnId, content: '', timestamp: '2026-03-06T15:00:00Z' })],
      segments,
    })))

    expect(record.orderedTurnIds).toEqual([turnId])
    expect(record.orderedSegmentIdsByTurnId[turnId]).toEqual(expectedSegmentIds)
    expect(record.timelineEntryIds).toEqual(expectedSegmentIds)
    expect(record.timelineEntryIdsByTurnId[turnId]).toEqual(expectedSegmentIds)
  })

  it('removes normalized conversations and summaries when a project cache is deleted', () => {
    const projectSnapshot = buildSnapshot()
    const otherSnapshot = buildSnapshot({
      conversation_id: 'conversation-other',
      project_path: '/tmp/other-project',
      title: 'Other project',
      turns: [buildTurn({ id: 'turn-other', content: 'Other project message.' })],
      segments: [buildSegment({ id: 'segment-other', turn_id: 'turn-other', content: 'Other project message.' })],
    })
    const cacheWithBothProjects = open(otherSnapshot, open(projectSnapshot))

    const result = removeProjectFromCache(cacheWithBothProjects, projectSnapshot.project_path)

    expect(result.conversationsById[projectSnapshot.conversation_id]).toBeUndefined()
    expect(result.summariesByProjectPath[projectSnapshot.project_path]).toBeUndefined()
    expect(result.conversationsById[otherSnapshot.conversation_id]?.project_path).toBe(otherSnapshot.project_path)
    expect(result.summariesByProjectPath[otherSnapshot.project_path]).toHaveLength(1)
  })
})

describe('the row reducer', () => {
  it('updates the thread list title from a newer conversation row, as a generated title arrives', () => {
    const titled = live(open(buildSnapshot()), [conversation(1, { title: 'Generated title' })])
    expect(recordOf(titled).title).toBe('Generated title')
    expect(titled.summariesByProjectPath[PROJECT].map((summary) => summary.title)).toEqual(['Generated title'])
  })

  it('ignores equal and older revisions, so replays are idempotent', () => {
    const cache = open(buildSnapshot({ revision: 4 }))
    const equal = ack(cache, [turn(buildTurn({ content: 'Equal revision must not replace.' }), 4)], { watermark: '5' })
    expect(recordOf(equal).turnsById['turn-assistant']?.content).toBe('I prepared the run request for review.')
    const olderRow = live(equal, [turn(buildTurn({ content: 'Outdated.' }), 3)], { from_revision: '2', watermark: '5' })
    expect(recordOf(olderRow).turnsById['turn-assistant']?.content).toBe('I prepared the run request for review.')

    const applied = live(olderRow, [turn(buildTurn({ content: 'Newer.' }), 6)])
    expect(recordOf(applied).turnsById['turn-assistant']?.content).toBe('Newer.')
    // The same batch again publishes nothing new.
    expect(live(applied, [turn(buildTurn({ content: 'Newer.' }), 6)])).toBe(applied)
    expect(ack(applied, [turn(buildTurn({ content: 'Newer.' }), 6)])).toBe(applied)
  })

  it('applies a whole batch in one published state', () => {
    const cache = open(buildSnapshot())
    const next = live(cache, [
      conversation(2, { title: 'Both at once' }),
      turn(buildTurn({ id: 'turn-next', role: 'user', content: 'Next.', timestamp: '2026-03-06T15:02:00Z' }), 2),
      segment(buildSegment({ content: 'Edited.' }), 1),
    ])
    const record = recordOf(next)
    expect(record.title).toBe('Both at once')
    expect(record.orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'turn-next'])
    expect(record.segmentsById['segment-assistant']?.content).toBe('Edited.')
    expect(record.watermark).toBe('2')
  })

  it('rejects a whole batch with an invalid row', () => {
    expect(() => parseConversationBatchResponse(conversationBatch([
      turn(buildTurn({ content: 'Fine.' }), 2),
      { kind: 'turn', conversation_id: ID, id: 'broken', position: '1', revision: 'two', deleted: false, payload: {} },
    ]))).toThrow()
  })

  it('takes chat_mode from the conversation row a mode change commits with', () => {
    const cache = live(open(buildSnapshot()), [
      conversation(1, { chat_mode: 'plan' }),
      turn({ id: 'turn-mode-1', role: 'system', kind: 'mode_change', status: 'complete', content: 'plan', timestamp: '2026-03-06T15:01:30Z', artifact_id: null }, 1),
    ])
    expect(recordOf(cache).chat_mode).toBe('plan')
    expect(getConversationTimelineEntries(recordOf(cache))).toContainEqual(expect.objectContaining({ kind: 'mode_change', mode: 'plan' }))
  })

  it('appends new turns and updates existing turns in place', () => {
    const appended = live(open(buildSnapshot()), [turn(buildTurn({
      id: 'turn-assistant-2',
      content: '',
      status: 'streaming',
      timestamp: '2026-03-06T15:01:30Z',
    }), 1)])

    expect(recordOf(appended).orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'turn-assistant-2'])
    expect(getConversationTimelineEntries(recordOf(appended))).toContainEqual(expect.objectContaining({
      id: 'turn-assistant-2:thinking:placeholder',
      content: '',
      presentation: 'thinking',
    }))

    const updated = live(appended, [turn(buildTurn({
      id: 'turn-assistant-2',
      content: 'The appended turn is now complete.',
      status: 'complete',
      timestamp: '2026-03-06T15:01:30Z',
    }), 2)])

    expect(recordOf(updated).orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'turn-assistant-2'])
    expect(recordOf(updated).turnsById['turn-assistant-2']?.content).toBe('The appended turn is now complete.')
    expect(getConversationTimelineEntries(recordOf(updated))).toContainEqual(expect.objectContaining({
      id: 'turn-assistant-2:default:placeholder',
      content: 'The appended turn is now complete.',
      presentation: 'default',
    }))
  })

  it('keeps queued turns in the order their positions give, as the server moves them', () => {
    const queued = (id: string, status: ConversationTurnResponse['status']) => buildTurn({ id, role: 'user', content: id, status })
    let cache = open(buildSnapshot())
    cache = live(cache, [turn(queued('second', 'queued'), 1, 3e12), turn(queued('third', 'queued'), 2, 4e12)])
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'second', 'third'])
    // The server starts `second`: its reply follows it, and `third` moves behind the reply.
    cache = live(cache, [
      turn(queued('second', 'complete'), 3, 3e12),
      turn(buildTurn({ id: 'reply', status: 'streaming', content: '', parent_turn_id: 'second' }), 4, 4e12),
      turn(queued('third', 'queued'), 5, 5e12),
    ])

    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-user', 'turn-assistant', 'second', 'reply', 'third'])
    expect(getConversationTimelineEntries(recordOf(cache)).filter((entry) => entry.role === 'user').map((entry) => [entry.id, entry.kind === 'message' && entry.status]))
      .toEqual([['turn-user', 'complete'], ['second', 'complete'], ['third', 'queued']])
  })

  it('patches streaming segment content on the normalized turn timeline', () => {
    const streaming = buildSegment({ id: 'assistant-stream', status: 'running', content: 'Hel' })
    const cache = live(open(buildSnapshot({ segments: [streaming] })), [segment({ ...streaming, content: 'Hello from streaming.' }, 1)])

    expect(getConversationTimelineEntries(recordOf(cache))).toContainEqual(expect.objectContaining({
      id: 'assistant-stream',
      kind: 'message',
      content: 'Hello from streaming.',
    }))
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-user', 'turn-assistant'])
  })

  it('applies artifacts that arrive in the same batch as their segment', () => {
    const cache = live(open(buildSnapshot({ turns: [buildSnapshot().turns[0], buildTurn({ content: '' })], segments: [] })), [
      segment(buildSegment({ id: 'segment-plan-1', kind: 'plan', content: 'Do the work incrementally.', artifact_id: 'plan-1' }), 1),
      artifactRow(ID, 'proposed_plans', buildProposedPlan(), '1', 1),
      artifactRow(ID, 'flow_run_requests', buildFlowRunRequest(), '1', 1),
      artifactRow(ID, 'flow_launches', buildFlowLaunch(), '1', 1),
    ])
    const record = recordOf(cache)

    expect(record.proposedPlansById['plan-1']?.title).toBe('Implementation plan')
    expect(record.flowRunRequestsById['request-1']?.summary).toBe('Launch the implementation flow.')
    expect(record.flowLaunchesById['launch-1']?.status).toBe('pending')
    expect(getConversationTimelineEntries(record)).toContainEqual(expect.objectContaining({
      id: 'segment-plan-1',
      kind: 'plan',
      artifactId: 'plan-1',
    }))
  })

  it('updates an artifact from a newer row, as a request is approved', () => {
    const cache = open(buildSnapshot({
      segments: [buildSegment({ id: 'segment-request-1', kind: 'flow_run_request', role: 'system', content: 'Review request', artifact_id: 'request-1' })],
      flow_run_requests: [buildFlowRunRequest()],
    }))
    const approved = live(cache, [artifactRow(ID, 'flow_run_requests', buildFlowRunRequest({ status: 'approved', run_id: 'run-1' }), '1', 1)])

    expect(recordOf(approved).flowRunRequestsById['request-1']).toMatchObject({ status: 'approved', run_id: 'run-1' })
    expect(getConversationTimelineEntries(recordOf(approved))).toContainEqual(expect.objectContaining({
      id: 'segment-request-1',
      kind: 'flow_run_request',
      artifactId: 'request-1',
    }))
  })

  it('removes tombstoned segments from the timeline', () => {
    const cache = open(buildSnapshot({
      segments: [
        buildSegment({ id: 'assistant-final', content: 'Final answer.', order: 1 }),
        buildSegment({ id: 'segment-agent-event-turn-1-turn_completed-6', kind: 'tool_call', status: 'running', content: '', order: 2 }),
      ],
    }))
    const result = live(cache, [tombstoneRow('segment', ID, 'segment-agent-event-turn-1-turn_completed-6', '2', { turn_id: 'turn-assistant' })])

    expect(recordOf(result).segmentsById['segment-agent-event-turn-1-turn_completed-6']).toBeUndefined()
    expect(timelineIds(result)).not.toContain('segment-agent-event-turn-1-turn_completed-6')
    expect(timelineIds(result)).toContain('assistant-final')
  })

  it('keeps a tombstone so a stale page or acknowledgement cannot bring the row back', () => {
    const cache = live(open(buildSnapshot({ revision: 1 })), [tombstoneRow('segment', ID, 'segment-assistant', '5', { turn_id: 'turn-assistant' })])
    expect(recordOf(cache).rows['segment\u0000conversation-1\u0000segment-assistant']).toMatchObject({ deleted: true, revision: '5' })

    const staleAck = ack(cache, [segment(buildSegment({ content: 'Back from the dead.' }), 4)], { watermark: '4' })
    expect(recordOf(staleAck).segmentsById['segment-assistant']).toBeUndefined()

    const withOlder = older({ ...staleAck, conversationsById: { [ID]: { ...recordOf(staleAck), olderCursor: 'cursor-1' } } },
      buildSnapshot({ revision: 3, turns: [buildSnapshot().turns[1]] }), { watermark: '4' })
    expect(recordOf(withOlder).segmentsById['segment-assistant']).toBeUndefined()
  })

  it('drops every child of a deleted conversation and ignores late children', () => {
    const cache = live(open(buildSnapshot({ revision: 1 })), [tombstoneRow('conversation', ID, ID, '3')])
    const record = recordOf(cache)
    expect(record.deleted).toBe(true)
    expect(record.orderedTurnIds).toEqual([])
    expect(Object.keys(record.rows)).toHaveLength(1)
    expect(cache.unavailableConversationIds?.[ID]).toBe(true)
    expect(cache.summariesByProjectPath[PROJECT]).toEqual([])

    const late = ack(cache, [turn(buildTurn({ id: 'turn-late', content: 'Too late.' }), 2)], { watermark: '3' })
    expect(recordOf(late).orderedTurnIds).toEqual([])
    const lateNewer = live(late, [turn(buildTurn({ id: 'turn-later', content: 'Later.' }), 4)])
    expect(recordOf(lateNewer).orderedTurnIds).toEqual([])
  })

  it('ignores live batches for a conversation it has not loaded', () => {
    expect(live(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, [turn(buildTurn(), 1)])).toBe(EMPTY_PROJECT_CONVERSATION_CACHE_STATE)
  })

  it('starts a conversation from a mutation result, as creating a thread does', () => {
    const cache = ack(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, [conversation(1, { title: 'New thread' })])
    expect(recordOf(cache)).toMatchObject({ title: 'New thread', revision: '1', watermark: null, olderCursor: null })
    expect(cache.summariesByProjectPath[PROJECT]).toEqual([expect.objectContaining({ conversation_id: ID, revision: '1' })])
  })
})

describe('the live cursor', () => {
  it('advances on every live batch, even an empty one, and across revision gaps', () => {
    let cache = open(buildSnapshot({ revision: 3 }))
    expect(recordOf(cache).watermark).toBe('3')
    // A filtered stream skips other conversations' revisions; nothing is missing.
    cache = live(cache, [], { from_revision: '3', watermark: '9' })
    expect(recordOf(cache).watermark).toBe('9')
    cache = live(cache, [turn(buildTurn({ content: 'After a gap.' }), 14)], { from_revision: '9', watermark: '20' })
    expect(recordOf(cache).watermark).toBe('20')
    expect(recordOf(cache).turnsById['turn-assistant']?.content).toBe('After a gap.')
  })

  it('never moves on a mutation result or an older page', () => {
    let cache = open(buildSnapshot({ revision: 3 }), undefined, { older_cursor: 'cursor-1' })
    cache = ack(cache, [conversation(8, { title: 'Acknowledged' })])
    expect(recordOf(cache).watermark).toBe('3')
    cache = older(cache, buildSnapshot({ revision: 2, turns: [], segments: [] }), { watermark: '12' })
    expect(recordOf(cache).watermark).toBe('3')
  })

  it('validates that a catch-up batch holds only rows after its start, up to its watermark', () => {
    expect(() => parseConversationBatchResponse(conversationBatch([turn(buildTurn(), 3)], { from_revision: '3', watermark: '4' }))).toThrow()
    expect(() => parseConversationBatchResponse(conversationBatch([turn(buildTurn(), 5)], { from_revision: '3', watermark: '4' }))).toThrow()
    // A mutation result may return an unchanged row at or below its start.
    expect(parseConversationBatchResponse(conversationBatch([conversation(3)], { from_revision: '7', watermark: '7' }), undefined, 'mutation').rows).toHaveLength(1)
  })

  it('stops applying live batches after a reset until the next window', () => {
    let cache = resetConversationInCache(open(buildSnapshot({ revision: 3 })), ID)
    expect(recordOf(cache)).toMatchObject({ resetting: true, watermark: null })
    cache = live(cache, [turn(buildTurn({ content: 'Ignored while resetting.' }), 4)])
    expect(recordOf(cache).turnsById['turn-assistant']?.content).toBe('I prepared the run request for review.')
    cache = open(buildSnapshot({ revision: 6 }), cache)
    expect(recordOf(cache)).toMatchObject({ resetting: false, watermark: '6' })
  })
})

describe('windows and older pages', () => {
  const history = (count: number, offset = 0) => Array.from({ length: count }, (_, index) => buildTurn({
    id: `turn-${index + offset}`,
    role: 'user',
    content: `Message ${index + offset}`,
    timestamp: new Date(Date.parse('2026-03-06T15:00:00Z') + (index + offset) * 1000).toISOString(),
  }))

  it('adds an older page before the window without touching newer rows', () => {
    let cache = open(buildSnapshot({ revision: 5, turns: history(2, 2), segments: [] }), undefined, { older_cursor: 'cursor-1' })
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-2', 'turn-3'])
    cache = live(cache, [turn({ ...history(1, 3)[0], content: 'Edited live.' }, 7)])
    cache = older(cache, buildSnapshot({ revision: 4, turns: history(4), segments: [] }), { watermark: '6', older_cursor: null })
    const record = recordOf(cache)
    expect(record.orderedTurnIds).toEqual(['turn-0', 'turn-1', 'turn-2', 'turn-3'])
    expect(record.turnsById['turn-3']?.content).toBe('Edited live.')
    expect(record.olderCursor).toBeNull()
  })

  it('keeps an older turn a delta changed out of view until its page loads', () => {
    let cache = open(buildSnapshot({ revision: 5, turns: history(2, 2), segments: [] }), undefined, { older_cursor: 'cursor-1' })
    cache = live(cache, [turn({ ...history(1)[0], content: 'Changed while unloaded.' }, 6)])
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-2', 'turn-3'])
    cache = older(cache, buildSnapshot({ revision: 4, turns: history(2), segments: [] }), { older_cursor: null })
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-0', 'turn-1', 'turn-2', 'turn-3'])
    expect(recordOf(cache).turnsById['turn-0']?.content).toBe('Changed while unloaded.')
  })

  it('shows pinned turns outside the window', () => {
    const cache = open(buildSnapshot({ revision: 5, turns: history(2, 2), segments: [] }), undefined, { older_cursor: 'cursor-1' })
    const pinned = live(cache, [turn({ ...history(1)[0], status: 'queued' }, 6)])
    expect(recordOf(pinned).orderedTurnIds).toEqual(['turn-2', 'turn-3'])
    const withPin = applyConversationPageToCache(cache, {
      ...page(buildSnapshot({ revision: 6, turns: [...history(1), ...history(2, 2)], segments: [] }), { older_cursor: 'cursor-1' }),
      turn_ids: ['turn-2', 'turn-3'],
      pinned_turn_ids: ['turn-0'],
    }, 'window')
    expect(recordOf(withPin).orderedTurnIds).toEqual(['turn-0', 'turn-2', 'turn-3'])
  })

  it('discards loaded history and cursors on a window reset but keeps rows newer than its read', () => {
    let cache = open(buildSnapshot({ revision: 5, turns: history(2, 2), segments: [] }), undefined, { older_cursor: 'cursor-1' })
    cache = older(cache, buildSnapshot({ revision: 4, turns: history(2), segments: [] }), { older_cursor: 'cursor-0' })
    cache = live(cache, [turn(history(1, 9)[0], 9)], { from_revision: '5', watermark: '9' })
    // A newest window read at 8, before the live row at 9, replaces everything older.
    cache = open(buildSnapshot({ revision: 8, turns: history(2, 3), segments: [] }), cache, { older_cursor: 'cursor-2' })
    const record = recordOf(cache)
    expect(record.orderedTurnIds).toEqual(['turn-3', 'turn-4', 'turn-9'])
    expect(record.olderCursor).toBe('cursor-2')
    expect(record.watermark).toBe('9')
    expect(record.rows['turn\u0000conversation-1\u0000turn-0']).toBeUndefined()
  })

  it('never lets a page read before a live update overwrite it', () => {
    let cache = open(buildSnapshot({ revision: 3 }))
    cache = live(cache, [turn(buildTurn({ content: 'Live and newer.' }), 6)], { from_revision: '3', watermark: '6' })
    cache = open(buildSnapshot({ revision: 5, turns: [buildSnapshot().turns[0], buildTurn({ content: 'Page read earlier.' })] }), cache)
    expect(recordOf(cache).turnsById['turn-assistant']?.content).toBe('Live and newer.')
    expect(recordOf(cache).watermark).toBe('6')
  })

  it('starts over on a window from another store generation', () => {
    let cache = open(buildSnapshot({ revision: 30, turns: history(3), segments: [] }), undefined, { older_cursor: 'cursor-1' })
    cache = open(buildSnapshot({ revision: 2, turns: history(1, 5), segments: [] }), cache, { generation: 'generation-2' })
    expect(recordOf(cache)).toMatchObject({ generation: 'generation-2', watermark: '2', olderCursor: null })
    expect(recordOf(cache).orderedTurnIds).toEqual(['turn-5'])
    // A batch from the previous generation no longer applies.
    expect(live(cache, [turn(history(1)[0], 31)])).toBe(cache)
  })

  it('reuses the timeline entries of turns a batch leaves unchanged', () => {
    const cache = open(buildSnapshot())
    const before = recordOf(cache)
    const after = recordOf(live(cache, [turn(buildTurn({ id: 'turn-next', role: 'user', content: 'Next.', timestamp: '2026-03-06T15:02:00Z' }), 1)]))
    expect(after.timelineEntriesById['segment-assistant']).toBe(before.timelineEntriesById['segment-assistant'])
    expect(after.timelineEntriesById['turn-user']).toBe(before.timelineEntriesById['turn-user'])
  })
})

describe('acknowledgements', () => {
  const sent = buildTurn({ id: 'turn-client-1', role: 'user', content: 'Hello.', status: 'complete', timestamp: '2026-03-06T15:02:00Z' })
  const reply = buildTurn({ id: 'turn-reply-1', status: 'pending', content: '', timestamp: '2026-03-06T15:02:01Z', parent_turn_id: 'turn-client-1' })

  it('renders a resent acceptance once', () => {
    const accepted = ack(open(buildSnapshot()), [conversation(1), turn(sent, 1), turn(reply, 1)], { watermark: '1' })
    expect(ack(accepted, [conversation(1), turn(sent, 1), turn(reply, 1)], { watermark: '1' })).toBe(accepted)
    expect(timelineIds(accepted).filter((id) => id === 'turn-client-1')).toHaveLength(1)
  })

  it('keeps a live settings change that overtook the settings response', () => {
    let cache = open(buildSnapshot({ revision: 5 }))
    cache = live(cache, [conversation(7, { chat_mode: 'plan' })], { from_revision: '5', watermark: '7' })
    cache = ack(cache, [conversation(6, { chat_mode: 'chat' })], { watermark: '6' })
    expect(recordOf(cache)).toMatchObject({ chat_mode: 'plan', revision: '7' })
  })

  it('applies an unchanged-settings response without changing anything', () => {
    const cache = open(buildSnapshot({ revision: 5 }))
    expect(ack(cache, [conversation(5)], { from_revision: '9', watermark: '9' })).toBe(cache)
  })
})

describe('summary snapshots', () => {
  const summary = (id: string, projectPath: string, revision: number, overrides: Record<string, unknown> = {}) => ({
    conversation_id: id, project_path: projectPath, title: id, created_at: '2026-03-06T15:00:00Z', updated_at: `2026-03-06T15:0${revision % 10}:00Z`, revision, ...overrides,
  })
  const snapshot = (summaries: Record<string, unknown>[], projectPath: string | null, watermark: string, generation?: string) => (
    parseConversationSummarySnapshotResponse(summarySnapshot(summaries, { project_path: projectPath, watermark, generation }))
  )
  const ids = (cache: ProjectConversationCacheState, projectPath: string) => (cache.summariesByProjectPath[projectPath] ?? []).map((item) => item.conversation_id)

  it('initializes an empty list and lists across projects', () => {
    const empty = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([], '/a', '4'), 'http')
    expect(empty.summariesByProjectPath).toEqual({ '/a': [] })
    expect(empty.summaryCursor).toEqual({ generation: 'generation-test', watermark: '4' })

    const all = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([summary('a1', '/a', 1), summary('b1', '/b', 2), summary('a2', '/a', 3)], null, '5'), 'live')
    expect(ids(all, '/a')).toEqual(['a2', 'a1'])
    expect(ids(all, '/b')).toEqual(['b1'])
    expect(all.summariesByProjectPath['/a'][0]).toMatchObject({ revision: '3', title: 'a2' })
  })

  it('replaces only the snapshot\'s own project and leaves open transcripts alone', () => {
    let cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([summary('a1', '/a', 1), summary('b1', '/b', 2)], null, '2'), 'live')
    cache = open(buildSnapshot({ conversation_id: 'a1', project_path: '/a', revision: 1 }), cache)
    cache = applySummarySnapshotToCache(cache, snapshot([summary('a2', '/a', 3)], '/a', '3'), 'http')
    expect(ids(cache, '/a')).toEqual(['a2'])
    expect(ids(cache, '/b')).toEqual(['b1'])
    expect(cache.conversationsById.a1).toBeDefined()
  })

  it('ignores what a stale response predates: newer rows, newer threads and deletions', () => {
    let cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([summary('a1', '/a', 1), summary('a2', '/a', 2)], null, '2'), 'live')
    cache = applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([
      conversationRow(summary('a1', '/a', 6, { title: 'Renamed live' })),
      conversationRow(summary('a3', '/a', 7)),
      tombstoneRow('conversation', 'a2', 'a2', '8'),
    ], { from_revision: '2', watermark: '8' })))
    expect(ids(cache, '/a')).toEqual(['a3', 'a1'])

    // A response read at 5 lists a1's older title and a2 before its deletion, and not a3.
    cache = applySummarySnapshotToCache(cache, snapshot([summary('a1', '/a', 1), summary('a2', '/a', 2)], '/a', '5'), 'http')
    expect(ids(cache, '/a')).toEqual(['a3', 'a1'])
    expect(cache.summariesByProjectPath['/a'].find((item) => item.conversation_id === 'a1')?.title).toBe('Renamed live')
    // An HTTP snapshot only lowers the live cursor, so the feed still catches up from it.
    expect(cache.summaryCursor?.watermark).toBe('5')
  })

  it('keeps the live cursor when an HTTP snapshot is newer than it', () => {
    const cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([], null, '3'), 'live')
    expect(applySummarySnapshotToCache(cache, snapshot([], '/a', '9'), 'http').summaryCursor?.watermark).toBe('3')
  })

  it('advances the cursor on an empty batch and never moves it back', () => {
    let cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([], null, '3'), 'live')
    cache = applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([], { from_revision: '3', watermark: '11' })))
    expect(cache.summaryCursor?.watermark).toBe('11')
    cache = applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([], { from_revision: '3', watermark: '4' })))
    expect(cache.summaryCursor?.watermark).toBe('11')
  })

  it('replaces a standing role\'s placeholder with its real conversation', () => {
    const role = { name: 'coordinator', title: 'Coordinator', state: 'idle' }
    let cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([summary('role-1', '/a', 0, { standing_role: role, placeholder: true, title: 'Coordinator' })], null, '3'), 'live')
    expect(cache.summariesByProjectPath['/a']).toEqual([expect.objectContaining({ revision: '0', standing_role: role })])
    cache = applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([conversationRow(summary('role-1', '/a', 4, { standing_role: role, title: 'Coordinator' }))], { from_revision: '3' })))
    expect(cache.summariesByProjectPath['/a']).toEqual([expect.objectContaining({ revision: '4', standing_role: role })])
  })

  it('discards revisions from another store generation', () => {
    let cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, snapshot([summary('a1', '/a', 40)], null, '40'), 'live')
    cache = applySummarySnapshotToCache(cache, snapshot([summary('a1', '/a', 2, { title: 'Fresh store' })], null, '2', 'generation-2'), 'live')
    expect(cache.summariesByProjectPath['/a']).toEqual([expect.objectContaining({ title: 'Fresh store', revision: '2' })])
  })
})

describe('thread-list summaries the Overview reads', () => {
  const projectPath = '/tmp/project-contract-behavior'
  const run = (run_id: string, status: string, ended_at: string) => ({
    run_id, flow_name: 'implementation.dot', status, working_directory: projectPath, project_path: projectPath, model: 'gpt', started_at: '2026-03-06T15:00:30Z', ended_at,
  })
  const chats = (cache: typeof EMPTY_PROJECT_CONVERSATION_CACHE_STATE) => cache.summariesByProjectPath[projectPath] ?? []

  it('keep a chat\'s launched runs through window and committed live updates', () => {
    const runs = [run('run', 'completed', '2026-03-06T16:00:00Z')]

    const afterWindow = open(buildSnapshot({
      revision: 1,
      flow_launches: [buildFlowLaunch({ status: 'launched', run_id: 'run' }), buildFlowLaunch({ id: 'launch-pending' })],
    }))
    expect(chats(afterWindow)[0]?.launched_run_ids).toEqual(['run'])
    expect(sourceOf({ kind: 'run', id: 'run' }, chats(afterWindow), [])).toEqual({ kind: 'chat', chat: chats(afterWindow)[0] })
    const last = lastChat(chats(afterWindow), runs, [])
    expect(last?.at).toBe(Date.parse('2026-03-06T16:00:00Z'))
    expect(tally(last!.started)).toEqual({ completed: 1 })

    // A message turn carries no launches; the conversation row of one that launches another run adds it.
    const afterTurn = live(afterWindow, [turn(buildTurn({ id: 'turn-next', content: 'More.', timestamp: '2026-03-06T15:02:00Z' }), 2)])
    expect(chats(afterTurn)[0]?.launched_run_ids).toEqual(['run'])

    const afterLaunch = live(afterTurn, [
      conversation(3, { launched_run_ids: ['run', 'run-2'] }),
      segment(buildSegment({ id: 'segment-launch-2', turn_id: 'turn-next', kind: 'flow_launch', artifact_id: 'launch-2' }), 3),
      artifactRow(ID, 'flow_launches', buildFlowLaunch({ id: 'launch-2', status: 'launched', run_id: 'run-2' }), '3', 3),
    ])
    const latest = chats(afterLaunch)
    expect(latest[0]?.launched_run_ids).toEqual(['run', 'run-2'])
    expect(sourceOf({ kind: 'run', id: 'run' }, latest, [])).toEqual({ kind: 'chat', chat: latest[0] })
    const withSecond = lastChat(latest, [...runs, run('run-2', 'failed', '2026-03-06T17:00:00Z')], [])
    expect(withSecond?.at).toBe(Date.parse('2026-03-06T17:00:00Z'))
    expect(tally(withSecond!.started)).toEqual({ completed: 1, failed: 1 })
  })
})

describe('a standing role\'s thread-list row', () => {
  const role = { name: 'coordinator', title: 'Coordinator', state: 'idle' as const }

  it('keeps its role and title through opening it and later live updates', () => {
    const seeded = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, parseConversationSummarySnapshotResponse(summarySnapshot([{
      conversation_id: ID, project_path: PROJECT, title: 'Coordinator', created_at: '', updated_at: '', revision: 0, standing_role: role, placeholder: true,
    }], { watermark: '0' })), 'live')
    // Opening the placeholder loads a fresh shell titled like any new thread.
    const opened = open(buildSnapshot({ revision: 1, title: 'New thread', turns: [], segments: [] }), seeded)
    expect(opened.summariesByProjectPath[PROJECT]).toEqual([expect.objectContaining({ title: 'Coordinator', standing_role: role })])

    const streamed = live(opened, [conversation(2, { title: 'New thread', last_message_preview: 'Hello.' }), turn(buildTurn({ id: 'turn-next', role: 'user', content: 'Hello.' }), 2)])
    expect(streamed.summariesByProjectPath[PROJECT]).toEqual([
      expect.objectContaining({ title: 'Coordinator', standing_role: role, last_message_preview: 'Hello.' }),
    ])
  })
})

describe('a summary\'s running flag', () => {
  const summaryOf = (cache: typeof EMPTY_PROJECT_CONVERSATION_CACHE_STATE) => cache.summariesByProjectPath[PROJECT]?.[0]

  it('follows the conversation row the server sends', () => {
    expect(summaryOf(open(buildSnapshot({ revision: 1, turns: [buildTurn({ status: 'streaming' })] })))?.running).toBe(true)
    expect(summaryOf(open(buildSnapshot({ revision: 1 })))?.running).toBe(false)
  })

  it('leaves a newer server summary in place when an older window is applied', () => {
    const seeded = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, parseConversationSummarySnapshotResponse(summarySnapshot([{
      conversation_id: ID, project_path: PROJECT, title: 'Contract behavior', created_at: '2026-03-06T15:00:00Z', updated_at: '2026-03-06T15:05:00Z', revision: 5, running: true,
    }])), 'live')
    const applied = open(buildSnapshot({ revision: 4 }), seeded)
    expect(summaryOf(applied)).toBe(summaryOf(seeded))
  })
})

describe('applyTransientConversationEventToCache', () => {
  const buildDelta = (
    overrides: Partial<ConversationStreamDeltaEventResponse> = {},
  ): ConversationStreamDeltaEventResponse => ({
    type: 'stream_delta',
    conversation_id: ID,
    turn_id: 'turn-assistant',
    stream_sequence: 1,
    base_revision: 3,
    delta_kind: 'segment_delta',
    segment: buildSegment({
      status: 'streaming',
      completed_at: null,
      content: 'Partial stream',
    }),
    ...overrides,
  })

  const seededCache = (turnStatus: ConversationTurnResponse['status'] = 'streaming') => open(buildSnapshot({
    revision: 3,
    turns: [buildSnapshot().turns[0], buildTurn({ status: turnStatus })],
  }))

  it('applies segment deltas to the timeline without changing a row or revision', () => {
    const cache = seededCache()
    const result = applyTransientConversationEventToCache(cache, buildDelta())

    expect(result.status).toBe('applied')
    if (result.status !== 'applied') {
      return
    }
    expect(result.record.revision).toBe('3')
    expect(result.record.rows).toBe(recordOf(cache).rows)
    expect(result.record.segmentsById['segment-assistant']?.content).toBe('Partial stream')
    const entries = getConversationTimelineEntries(result.record)
    expect(entries.some((entry) => 'content' in entry && entry.content === 'Partial stream')).toBe(true)
  })

  it('applies turn deltas with streaming-content sanitization and no revision change', () => {
    const result = applyTransientConversationEventToCache(seededCache(), buildDelta({
      delta_kind: 'turn_delta',
      segment: undefined,
      turn: buildTurn({ status: 'streaming', content: 'Live turn content' }),
    }))

    expect(result.status).toBe('applied')
    if (result.status !== 'applied') {
      return
    }
    expect(result.record.revision).toBe('3')
    expect(result.record.turnsById['turn-assistant']?.content).toBe('Live turn content')
    expect(result.record.turnsById['turn-assistant']?.status).toBe('streaming')
  })

  it('keeps streamed turn text when a running turn commits without it', () => {
    const streamed = applyTransientConversationEventToCache(seededCache(), buildDelta({
      delta_kind: 'turn_delta',
      segment: undefined,
      turn: buildTurn({ status: 'streaming', content: 'Live turn content' }),
    }))
    const committed = live(streamed.cache, [turn(buildTurn({ status: 'streaming', content: '' }), 4)])
    expect(recordOf(committed).turnsById['turn-assistant']?.content).toBe('Live turn content')
    const finished = live(committed, [turn(buildTurn({ status: 'complete', content: 'Final.' }), 5)])
    expect(recordOf(finished).turnsById['turn-assistant']?.content).toBe('Final.')
  })

  it('applies deltas after an unrelated commit raised the conversation revision', () => {
    const titled = live(seededCache(), [conversation(5, { title: 'Stored title' })])
    const result = applyTransientConversationEventToCache(titled, buildDelta({ base_revision: 3 }))

    expect(result.status).toBe('applied')
    if (result.status !== 'applied') {
      return
    }
    expect(result.record.revision).toBe('5')
    expect(result.record.segmentsById['segment-assistant']?.content).toBe('Partial stream')
  })

  it('drops late deltas for a turn that is already complete or failed', () => {
    for (const status of ['complete', 'failed'] as const) {
      const cache = seededCache(status)
      const result = applyTransientConversationEventToCache(cache, buildDelta({ stream_sequence: 99 }))

      expect(result.status).toBe('dropped')
      expect(result.cache).toBe(cache)
    }
  })

  it('drops deltas whose stream sequence is not newer than the last applied for the turn', () => {
    const first = applyTransientConversationEventToCache(seededCache(), buildDelta({ stream_sequence: 2 }))
    expect(first.status).toBe('applied')
    for (const stream_sequence of [1, 2]) {
      const result = applyTransientConversationEventToCache(first.cache, buildDelta({
        stream_sequence,
        segment: buildSegment({ status: 'streaming', completed_at: null, content: 'Older' }),
      }))
      expect(result.status).toBe('dropped')
      expect(result.cache).toBe(first.cache)
    }
    const newer = applyTransientConversationEventToCache(first.cache, buildDelta({ stream_sequence: 3 }))
    expect(newer.status).toBe('applied')
  })

  it('keeps per-turn stream sequences across window, batch and project removal updates', () => {
    const first = applyTransientConversationEventToCache(seededCache(), buildDelta({ stream_sequence: 10 }))
    expect(first.status).toBe('applied')
    const unrelatedUpdates = [
      (cache: typeof first.cache) => open(buildSnapshot({ revision: 5, title: 'Stored title', turns: [buildTurn({ status: 'streaming' })] }), cache),
      (cache: typeof first.cache) => live(cache, [turn(buildTurn({ id: 'turn-user', role: 'user', content: 'Draft a plan.', timestamp: '2026-03-06T15:00:00Z' }), 5)]),
      (cache: typeof first.cache) => removeProjectFromCache(cache, '/tmp/other-project'),
    ]
    for (const update of unrelatedUpdates) {
      const cache = update(first.cache)
      const olderDelta = applyTransientConversationEventToCache(cache, buildDelta({
        stream_sequence: 9,
        segment: buildSegment({ status: 'streaming', completed_at: null, content: 'Older' }),
      }))
      expect(olderDelta.status).toBe('dropped')
      const newer = applyTransientConversationEventToCache(cache, buildDelta({ stream_sequence: 11 }))
      expect(newer.status).toBe('applied')
    }
  })

  it('keeps a running turn\'s live segments through a mid-turn commit until its terminal commit', () => {
    const commentary = buildSegment({ id: 'segment-commentary', order: 1, status: 'complete', content: 'Looking at the repo.' })
    const answer = buildSegment({ id: 'segment-answer', order: 2, status: 'streaming', completed_at: null, content: 'Partial answer' })
    const running = open(buildSnapshot({ revision: 3, turns: [buildTurn({ status: 'streaming', content: '' })], segments: [] }))
    const withCommentary = applyTransientConversationEventToCache(running, buildDelta({ segment: commentary }))
    expect(withCommentary.status).toBe('applied')
    const titled = live(withCommentary.cache, [conversation(4, { title: 'Stored title' })])
    expect(recordOf(titled).title).toBe('Stored title')
    const withAnswer = applyTransientConversationEventToCache(titled, buildDelta({ stream_sequence: 2, segment: answer }))
    expect(withAnswer.status).toBe('applied')
    if (withAnswer.status !== 'applied') {
      return
    }
    expect(withAnswer.record.orderedSegmentIdsByTurnId['turn-assistant']).toEqual(['segment-commentary', 'segment-answer'])
    expect(withAnswer.record.segmentsById['segment-commentary']?.content).toBe('Looking at the repo.')

    const finalAnswer = buildSegment({ id: 'segment-final', order: 2, content: 'Done.' })
    const completed = live(withAnswer.cache, [
      turn(buildTurn({ status: 'complete', content: 'Done.' }), 5),
      segment(commentary, 5),
      segment(finalAnswer, 5),
    ])
    const record = recordOf(completed)
    expect(record.orderedSegmentIdsByTurnId['turn-assistant']).toEqual(['segment-commentary', 'segment-final'])
    expect(record.segmentsById['segment-answer']).toBeUndefined()
    const late = applyTransientConversationEventToCache(completed, buildDelta({ stream_sequence: 3, segment: answer }))
    expect(late.status).toBe('dropped')
  })

  it('shows the committed turn, not what it streamed, when a window reopens a chat that finished meanwhile', () => {
    const streamed = applyTransientConversationEventToCache(seededCache(), buildDelta({ segment: buildSegment({ status: 'streaming', content: 'Half done.' }) }))
    expect(streamed.status).toBe('applied')
    const reopened = open(buildSnapshot({
      revision: 6,
      turns: [buildSnapshot().turns[0], buildTurn({ status: 'complete', content: 'Half done. All done.' })],
      segments: [buildSegment({ content: 'Half done. All done.' })],
    }), streamed.cache)
    expect(recordOf(reopened).segmentsById['segment-assistant']?.content).toBe('Half done. All done.')
    expect(recordOf(reopened).streamSegmentsById).toEqual({})
  })

  it('drops deltas for unknown conversations instead of buffering them', () => {
    const result = applyTransientConversationEventToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      buildDelta({ conversation_id: 'conversation-unknown' }),
    )

    expect(result.status).toBe('dropped')
    expect(result.cache).toBe(EMPTY_PROJECT_CONVERSATION_CACHE_STATE)
  })

  it('lets a later committed segment row win over prior transients', () => {
    const transient = applyTransientConversationEventToCache(seededCache(), buildDelta())
    expect(transient.status).toBe('applied')
    const committed = live(transient.cache, [segment(buildSegment({ content: 'Final committed content' }), 4)])

    expect(recordOf(committed).latestRevision).toBe('4')
    expect(recordOf(committed).segmentsById['segment-assistant']?.content).toBe('Final committed content')
  })

  it('merges token usage deltas onto the targeted turn', () => {
    const result = applyTransientConversationEventToCache(seededCache(), buildDelta({
      delta_kind: 'token_usage',
      segment: undefined,
      token_usage: { total: { inputTokens: 12, outputTokens: 5 } },
    }))

    expect(result.status).toBe('applied')
    if (result.status !== 'applied') {
      return
    }
    expect(result.record.revision).toBe('3')
    expect(result.record.turnsById['turn-assistant']?.token_usage).toEqual({
      total: { inputTokens: 12, outputTokens: 5 },
    })
  })
})

describe('applyConversationPage', () => {
  it('builds a record from a page alone, as a mission log does', () => {
    const record = applyConversationPage(undefined, page(buildSnapshot({ revision: 2 })), 'window')
    expect(record?.orderedTurnIds).toEqual(['turn-user', 'turn-assistant'])
    expect(applyConversationPage(undefined, page(buildSnapshot()), 'older')).toBeUndefined()
  })
})

describe('the server\'s spelling of a project path', () => {
  const canonical = { project_path: `/private${PROJECT}` }

  it('files a conversation and its summary under the project path the client opened it with', () => {
    const cache = applyConversationPageToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, page(buildSnapshot({ revision: 1, ...canonical })), 'window', PROJECT)
    expect(recordOf(cache).project_path).toBe(PROJECT)
    expect(cache.summariesByProjectPath).toEqual({ [PROJECT]: [expect.objectContaining({ conversation_id: ID, project_path: PROJECT })] })
    const later = live(cache, [conversation(2, { ...canonical, title: 'Renamed' })])
    expect(recordOf(later).project_path).toBe(PROJECT)
    expect(Object.keys(later.summariesByProjectPath)).toEqual([PROJECT])
  })

  it('scopes a project\'s summary snapshot by the path it was requested with', () => {
    const response = parseConversationSummarySnapshotResponse(summarySnapshot([
      { conversation_id: 'a1', title: 'a1', created_at: '', updated_at: '', revision: 3, ...canonical },
    ], { project_path: canonical.project_path, watermark: '3' }))
    const cache = applySummarySnapshotToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, response, 'http', PROJECT)
    expect(cache.summariesByProjectPath).toEqual({ [PROJECT]: [expect.objectContaining({ conversation_id: 'a1', project_path: PROJECT })] })
    // The all-projects feed keeps it where it is listed.
    const updated = applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([
      conversationRow({ conversation_id: 'a1', title: 'Renamed', created_at: '', updated_at: '', ...canonical }, '4'),
    ])))
    expect(updated.summariesByProjectPath[PROJECT]).toEqual([expect.objectContaining({ title: 'Renamed', project_path: PROJECT })])
  })
})

describe('a new thread', () => {
  const emptyWindow = (watermark: string) => parseConversationPageResponse({
    generation: 'generation-test', watermark, conversation_id: ID, rows: [], turn_ids: [], pinned_turn_ids: [], pinned_artifact_ids: [], older_cursor: null,
  })

  it('opens from an empty window and takes its first commit from the live feed', () => {
    let cache = applyConversationPageToCache(EMPTY_PROJECT_CONVERSATION_CACHE_STATE, emptyWindow('7'), 'window', PROJECT)
    expect(recordOf(cache)).toMatchObject({ watermark: '7', revision: '0', project_path: PROJECT, orderedTurnIds: [] })
    cache = live(cache, [], { from_revision: '7', watermark: '8' })
    expect(recordOf(cache).watermark).toBe('8')
    cache = live(cache, [conversation(9), turn(buildTurn({ id: 'turn-first', role: 'user', content: 'First.' }), 9)], { from_revision: '8' })
    expect(recordOf(cache)).toMatchObject({ revision: '9', watermark: '9', orderedTurnIds: ['turn-first'] })
    expect(cache.summariesByProjectPath[PROJECT]).toEqual([expect.objectContaining({ conversation_id: ID })])
  })

  it('stays unavailable when an empty window follows its disappearance', () => {
    const gone = { ...EMPTY_PROJECT_CONVERSATION_CACHE_STATE, unavailableConversationIds: { [ID]: true as const } }
    expect(applyConversationPageToCache(gone, emptyWindow('7'), 'window', PROJECT)).toBe(gone)
  })
})
