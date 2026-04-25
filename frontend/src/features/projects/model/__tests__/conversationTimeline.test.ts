import {
  buildConversationTimelineEntries,
  stabilizeConversationTimelineEntries,
} from '@/features/projects/model/conversationTimeline'
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

  it('renders plan segments as dedicated timeline entries', () => {
    const timeline = buildConversationTimelineEntries({
      ...snapshot,
      chat_mode: 'plan',
      segments: [
        {
          id: 'plan-segment',
          turn_id: 'turn-assistant',
          order: 1,
          kind: 'plan',
          role: 'assistant',
          status: 'complete',
          timestamp: '2026-03-10T10:00:12Z',
          updated_at: '2026-03-10T10:00:12Z',
          completed_at: '2026-03-10T10:00:12Z',
          content: '1. Capture the real session path.',
          artifact_id: null,
          error: null,
          source: null,
          tool_call: null,
        },
      ],
    }, null)

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'plan',
      content: '1. Capture the real session path.',
      artifactId: null,
    }))
  })

  it('renders context_compaction segments as inline system timeline entries', () => {
    const timeline = buildConversationTimelineEntries({
      ...snapshot,
      segments: [
        {
          id: 'context-compaction-segment',
          turn_id: 'turn-assistant',
          order: 1,
          kind: 'context_compaction',
          role: 'system',
          status: 'complete',
          timestamp: '2026-03-10T10:00:11Z',
          updated_at: '2026-03-10T10:00:11Z',
          completed_at: '2026-03-10T10:00:11Z',
          content: 'Context compacted to continue the turn.',
          artifact_id: null,
          error: null,
          source: {
            app_turn_id: 'app-turn-1',
          },
          tool_call: null,
        },
        ...snapshot.segments,
      ],
    }, null)

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'context_compaction',
      content: 'Context compacted to continue the turn.',
    }))
  })

  it('renders request_user_input segments as inline conversation request entries', () => {
    const timeline = buildConversationTimelineEntries({
      ...snapshot,
      segments: [
        {
          id: 'request-user-input-segment',
          turn_id: 'turn-assistant',
          order: 1,
          kind: 'request_user_input',
          role: 'system',
          status: 'pending',
          timestamp: '2026-03-10T10:00:11Z',
          updated_at: '2026-03-10T10:00:11Z',
          completed_at: null,
          content: 'Which path should I take?',
          artifact_id: null,
          error: null,
          source: {
            app_turn_id: 'app-turn-1',
            item_id: 'request-1',
          },
          tool_call: null,
          request_user_input: {
            request_id: 'request-1',
            status: 'pending',
            questions: [
              {
                id: 'path_choice',
                header: 'Path',
                question: 'Which path should I take?',
                question_type: 'MULTIPLE_CHOICE',
                options: [
                  {
                    label: 'Inline card',
                    description: 'Keep the request inline.',
                  },
                ],
                allow_other: true,
                is_secret: false,
              },
            ],
            answers: {},
            submitted_at: null,
          },
        },
        ...snapshot.segments,
      ],
    }, null)

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'request_user_input',
      content: 'Which path should I take?',
      requestUserInput: expect.objectContaining({
        requestId: 'request-1',
        status: 'pending',
      }),
    }))
  })

  it('preserves expired request_user_input status in timeline entries', () => {
    const timeline = buildConversationTimelineEntries({
      ...snapshot,
      segments: [
        {
          id: 'request-user-input-expired',
          turn_id: 'turn-assistant',
          order: 1,
          kind: 'request_user_input',
          role: 'system',
          status: 'failed',
          timestamp: '2026-03-10T10:00:11Z',
          updated_at: '2026-03-10T10:00:11Z',
          completed_at: '2026-03-10T10:00:12Z',
          content: 'Which path should I take?\nAnswer: Inline card',
          artifact_id: null,
          error: 'The requested input expired before the answer could be used.',
          source: {
            app_turn_id: 'app-turn-1',
            item_id: 'request-1',
          },
          tool_call: null,
          request_user_input: {
            request_id: 'request-1',
            status: 'expired',
            questions: [],
            answers: {
              path_choice: 'Inline card',
            },
            submitted_at: '2026-03-10T10:00:12Z',
          },
        },
        ...snapshot.segments,
      ],
    }, null)

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'request_user_input',
      status: 'failed',
      requestUserInput: expect.objectContaining({
        requestId: 'request-1',
        status: 'expired',
      }),
    }))
  })

})

describe('stabilizeConversationTimelineEntries', () => {
  it('reuses unchanged timeline entry objects across rebuilds for the same conversation', () => {
    const previousEntries = buildConversationTimelineEntries(snapshot, null)
    const nextEntries = buildConversationTimelineEntries({ ...snapshot }, null)

    const stabilized = stabilizeConversationTimelineEntries('conversation-1', nextEntries, {
      conversationId: 'conversation-1',
      entries: previousEntries,
    })

    expect(stabilized[0]).toBe(previousEntries[0])
    expect(stabilized[1]).toBe(previousEntries[1])
    expect(stabilized[2]).toBe(previousEntries[2])
    expect(stabilized[3]).toBe(previousEntries[3])
  })

  it('keeps changed streaming entries fresh while reusing unchanged siblings', () => {
    const streamingSnapshot: ConversationSnapshotResponse = {
      ...snapshot,
      turns: snapshot.turns.map((turn) => (
        turn.id === 'turn-assistant' ? { ...turn, status: 'streaming' } : turn
      )),
      segments: snapshot.segments.map((segment) => (
        segment.id === 'assistant-segment'
          ? { ...segment, status: 'running', content: 'Do' }
          : segment
      )),
    }
    const updatedStreamingSnapshot: ConversationSnapshotResponse = {
      ...streamingSnapshot,
      segments: streamingSnapshot.segments.map((segment) => (
        segment.id === 'assistant-segment'
          ? { ...segment, content: 'Done streaming.' }
          : segment
      )),
    }
    const previousEntries = buildConversationTimelineEntries(streamingSnapshot, null)
    const nextEntries = buildConversationTimelineEntries(updatedStreamingSnapshot, null)

    const stabilized = stabilizeConversationTimelineEntries('conversation-1', nextEntries, {
      conversationId: 'conversation-1',
      entries: previousEntries,
    })

    expect(stabilized[0]).toBe(previousEntries[0])
    expect(stabilized[1]).toBe(previousEntries[1])
    expect(stabilized[2]).toBe(previousEntries[2])
    expect(stabilized[3]).not.toBe(previousEntries[3])
    expect(stabilized[3]).toMatchObject({
      kind: 'message',
      status: 'streaming',
      content: 'Done streaming.',
    })
  })

  it('does not reuse matching entries when the conversation scope changes', () => {
    const previousEntries = buildConversationTimelineEntries(snapshot, null)
    const nextEntries = buildConversationTimelineEntries({
      ...snapshot,
      conversation_id: 'conversation-2',
      conversation_handle: 'thread-2',
    }, null)

    const stabilized = stabilizeConversationTimelineEntries('conversation-2', nextEntries, {
      conversationId: 'conversation-1',
      entries: previousEntries,
    })

    expect(stabilized[0]).not.toBe(previousEntries[0])
    expect(stabilized[1]).not.toBe(previousEntries[1])
    expect(stabilized[2]).not.toBe(previousEntries[2])
    expect(stabilized[3]).not.toBe(previousEntries[3])
  })
})
