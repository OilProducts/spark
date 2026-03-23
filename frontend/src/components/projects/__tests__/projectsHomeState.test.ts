import { applyConversationSnapshotToCache, EMPTY_PROJECT_CONVERSATION_CACHE_STATE } from '@/components/projects/projectsHomeState'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'

const buildSnapshot = (
  overrides: Partial<ConversationSnapshotResponse> = {},
): ConversationSnapshotResponse => ({
  schema_version: 4,
  conversation_id: 'conversation-1',
  conversation_handle: '',
  project_path: '/tmp/project-contract-behavior',
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
      content: 'I drafted a spec edit proposal for review.',
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
      content: 'I drafted a spec edit proposal for review.',
      artifact_id: null,
      error: null,
      tool_call: null,
      source: null,
    },
  ],
  event_log: [
    {
      message: 'Execution planning started (workflow-1) using contract-behavior.dot.',
      timestamp: '2026-03-06T15:01:00Z',
    },
  ],
  spec_edit_proposals: [
    {
      id: 'proposal-1',
      created_at: '2026-03-06T15:00:11Z',
      summary: 'Require explicit spec review before execution planning.',
      status: 'applied',
      changes: [],
      canonical_spec_edit_id: 'spec-edit-1',
      approved_at: '2026-03-06T15:01:00Z',
      git_branch: 'main',
      git_commit: 'abc123',
    },
  ],
  flow_run_requests: [],
  flow_launches: [],
  execution_cards: [],
  execution_workflow: {
    run_id: 'workflow-1',
    status: 'running',
    error: null,
    flow_source: 'contract-behavior.dot',
  },
  ...overrides,
})

describe('applyConversationSnapshotToCache', () => {
  it('accepts same-timestamp snapshots when event and workflow state advances', () => {
    const initialSnapshot = buildSnapshot()
    const cacheWithInitialSnapshot = applyConversationSnapshotToCache(
      EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
      initialSnapshot.project_path,
      initialSnapshot,
    ).cache

    const failedSnapshot = buildSnapshot({
      event_log: [
        ...initialSnapshot.event_log,
        {
          message: 'Execution planning failed: plan status endpoint unavailable.',
          timestamp: '2026-03-06T15:02:00Z',
        },
      ],
      execution_workflow: {
        run_id: 'workflow-1',
        status: 'failed',
        error: 'plan status endpoint unavailable.',
        flow_source: 'contract-behavior.dot',
      },
    })

    const result = applyConversationSnapshotToCache(
      cacheWithInitialSnapshot,
      failedSnapshot.project_path,
      failedSnapshot,
    )

    expect(result.applied).toBe(true)
    expect(result.cache.snapshotsByConversationId[failedSnapshot.conversation_id]?.event_log).toHaveLength(2)
    expect(result.cache.snapshotsByConversationId[failedSnapshot.conversation_id]?.execution_workflow.status).toBe('failed')
  })
})
