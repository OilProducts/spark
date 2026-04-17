import {
  applyConversationSnapshotToCache,
  applyConversationStreamEventToCache,
  EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
} from '@/features/projects/model/projectsHomeState'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'

const buildSnapshot = (
  overrides: Partial<ConversationSnapshotResponse> = {},
): ConversationSnapshotResponse => ({
  schema_version: 4,
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
    {
      id: 'turn-assistant',
      role: 'assistant',
      content: 'I prepared the run request for review.',
      timestamp: '2026-03-06T15:00:10Z',
      kind: 'message',
      status: 'complete',
      artifact_id: null,
    },
  ],
  segments: [
    {
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
    },
  ],
  event_log: [
    {
      message: 'Flow run request request-1 approved for launch.',
      timestamp: '2026-03-06T15:01:00Z',
    },
  ],
  flow_run_requests: [],
  flow_launches: [],
  ...overrides,
})

describe('applyConversationSnapshotToCache', () => {
  it('accepts same-timestamp snapshots when the event log advances', () => {
    const initialSnapshot = buildSnapshot()
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const updatedSnapshot = buildSnapshot({
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
    expect(result.cache.snapshotsByConversationId[updatedSnapshot.conversation_id]?.event_log).toHaveLength(2)
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

    expect(result.snapshot?.chat_mode).toBe('plan')
    expect(result.cache.snapshotsByConversationId[initialSnapshot.conversation_id]?.chat_mode).toBe('plan')
  })
})
