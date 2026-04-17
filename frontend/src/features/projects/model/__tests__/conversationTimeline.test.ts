import { buildConversationTimelineEntries } from '@/features/projects/model/conversationTimeline'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'

const snapshot: ConversationSnapshotResponse = {
  schema_version: 4,
  conversation_id: 'conversation-1',
  conversation_handle: 'thread-1',
  project_path: '/tmp/project-a',
  chat_mode: 'chat',
  title: 'Test conversation',
  created_at: '2026-03-10T10:00:00Z',
  updated_at: '2026-03-10T10:01:00Z',
  turns: [
    {
      id: 'turn-user',
      role: 'user',
      content: 'Do the thing.',
      timestamp: '2026-03-10T10:00:00Z',
      kind: 'message',
      status: 'complete',
      artifact_id: null,
    },
    {
      id: 'turn-assistant',
      role: 'assistant',
      content: '',
      timestamp: '2026-03-10T10:00:05Z',
      kind: 'message',
      status: 'complete',
      artifact_id: null,
    },
  ],
  segments: [
    {
      id: 'tool-segment',
      turn_id: 'turn-assistant',
      order: 1,
      kind: 'tool_call',
      role: 'assistant',
      status: 'complete',
      timestamp: '2026-03-10T10:00:10Z',
      updated_at: '2026-03-10T10:00:10Z',
      completed_at: '2026-03-10T10:00:12Z',
      content: '',
      artifact_id: null,
      error: null,
      source: null,
      tool_call: {
        id: 'tool-1',
        kind: 'command_execution',
        status: 'completed',
        title: 'Run tests',
        command: 'pytest -q',
        output: 'ok',
        file_paths: [],
      },
    },
    {
      id: 'assistant-segment',
      turn_id: 'turn-assistant',
      order: 2,
      kind: 'assistant_message',
      role: 'assistant',
      status: 'complete',
      timestamp: '2026-03-10T10:00:15Z',
      updated_at: '2026-03-10T10:00:15Z',
      completed_at: '2026-03-10T10:00:15Z',
      content: 'Done.',
      artifact_id: null,
      error: null,
      source: null,
      tool_call: null,
    },
  ],
  event_log: [],

  flow_run_requests: [],
  flow_launches: [],


}

describe('buildConversationTimelineEntries', () => {
  it('inserts a worked separator before the final assistant message after tool activity', () => {
    const timeline = buildConversationTimelineEntries(snapshot, null)

    expect(timeline.map((entry) => entry.kind)).toEqual(['message', 'tool_call', 'final_separator', 'message'])
    expect(timeline[2]).toMatchObject({
      kind: 'final_separator',
      label: 'Worked for 10s',
    })
  })

  it('shows the optimistic user message when no snapshot exists yet', () => {
    const timeline = buildConversationTimelineEntries(null, {
      conversationId: 'conversation-2',
      createdAt: '2026-03-10T12:00:00Z',
      message: 'Please start.',
    })

    expect(timeline).toEqual([
      expect.objectContaining({
        role: 'user',
        content: 'Please start.',
      }),
    ])
  })

  it('includes mode_change entries in chronological order', () => {
    const timeline = buildConversationTimelineEntries({
      ...snapshot,
      chat_mode: 'plan',
      turns: [
        {
          id: 'turn-mode',
          role: 'system',
          content: 'plan',
          timestamp: '2026-03-10T09:59:59Z',
          kind: 'mode_change',
          status: 'complete',
          artifact_id: null,
        },
        ...snapshot.turns,
      ],
    }, null)

    expect(timeline.slice(0, 2)).toEqual([
      expect.objectContaining({
        kind: 'mode_change',
        mode: 'plan',
      }),
      expect.objectContaining({
        kind: 'message',
        role: 'user',
        content: 'Do the thing.',
      }),
    ])
  })

})
