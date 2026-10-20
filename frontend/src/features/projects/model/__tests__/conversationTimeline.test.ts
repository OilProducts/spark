import { getConversationTimelineEntries, hydrateConversationRecordFromSnapshot } from '@/features/projects/model/projectsHomeState'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'
import { parseConversationSnapshotResponse } from '@/lib/api/conversationsApi'

const baseSnapshot: ConversationSnapshotResponse = {
  schema_version: 5,
  revision: 0,
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
        command: 'cargo test -p spark-cli',
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
  proposed_plans: [],
}

function buildTimeline(snapshot: ConversationSnapshotResponse) {
  const record = hydrateConversationRecordFromSnapshot(snapshot)
  return getConversationTimelineEntries(record)
}

describe('conversation timeline normalization', () => {
  it('preserves tool row, worked separator, and assistant message ordering from a hydrated snapshot', () => {
    const timeline = buildTimeline(baseSnapshot)

    expect(timeline.map((entry) => entry.kind)).toEqual(['message', 'tool_call', 'final_separator', 'message'])
    expect(timeline[2]).toMatchObject({
      kind: 'final_separator',
      label: 'Worked for 10s',
    })
  })

  it('shows a message steered into a running turn as a user message where it was delivered', () => {
    const [tool, answer] = baseSnapshot.segments
    const snapshot = parseConversationSnapshotResponse({
      ...baseSnapshot,
      segments: [
        tool,
        {
          id: 'segment-user-message-turn-steer',
          turn_id: 'turn-assistant',
          order: 2,
          kind: 'user_message',
          role: 'user',
          status: 'complete',
          timestamp: '2026-03-10T10:00:12Z',
          updated_at: '2026-03-10T10:00:12Z',
          content: 'Also cover the edge case.',
          attachments: [{ id: 'att-1', filename: 'shot.png', media_type: 'image/png', size_bytes: 3 }],
        },
        { ...answer, order: 3 },
      ],
    })
    const timeline = buildTimeline(snapshot!)

    expect(timeline.map((entry) => [entry.kind, entry.role])).toEqual([
      ['message', 'user'],
      ['tool_call', 'system'],
      ['message', 'user'],
      ['final_separator', 'system'],
      ['message', 'assistant'],
    ])
    expect(timeline[2]).toMatchObject({
      content: 'Also cover the edge case.',
      attachments: [{ id: 'att-1', filename: 'shot.png' }],
    })
  })

  it('places the worked separator immediately before the last assistant segment', () => {
    const narration = {
      ...baseSnapshot.segments[1],
      id: 'narration-segment',
      order: 1,
      content: 'Inspecting.',
    }
    const firstTool = { ...baseSnapshot.segments[0], order: 2 }
    const secondNarration = {
      ...baseSnapshot.segments[1],
      id: 'second-narration-segment',
      order: 3,
      content: 'Running tests.',
    }
    const secondTool = {
      ...baseSnapshot.segments[0],
      id: 'second-tool-segment',
      order: 4,
      tool_call: { ...baseSnapshot.segments[0].tool_call!, id: 'tool-2' },
    }
    const answer = { ...baseSnapshot.segments[1], order: 5, content: 'Done.' }

    const timeline = buildTimeline({
      ...baseSnapshot,
      segments: [narration, firstTool, secondNarration, secondTool, answer],
    })

    expect(timeline.map((entry) => entry.kind)).toEqual([
      'message',
      'message',
      'tool_call',
      'message',
      'tool_call',
      'final_separator',
      'message',
    ])
    expect(timeline[5]).toMatchObject({ kind: 'final_separator' })
  })

  it('renders yielded tool rows as neutral terminal work before the final answer', () => {
    const yieldedTool = {
      ...baseSnapshot.segments[0],
      id: 'yielded-tool-segment',
      order: 1,
      tool_call: {
        ...baseSnapshot.segments[0].tool_call!,
        id: 'exec-a',
        status: 'yielded' as const,
        completion_reason: 'turn_boundary_yield',
        output: 'partial output',
      },
    }
    const answer = { ...baseSnapshot.segments[1], order: 2 }
    const timeline = buildTimeline({
      ...baseSnapshot,
      segments: [yieldedTool, answer],
    })

    expect(timeline.map((entry) => entry.kind)).toEqual(['message', 'tool_call', 'final_separator', 'message'])
    const toolEntry = timeline[1]
    expect(toolEntry).toMatchObject({ kind: 'tool_call' })
    if (toolEntry.kind === 'tool_call') {
      expect(toolEntry.toolCall.status).toBe('yielded')
      expect(toolEntry.toolCall.completionReason).toBe('turn_boundary_yield')
      expect(toolEntry.toolCall.output).toBe('partial output')
    }
    // The final answer stays the last visible entry.
    expect(timeline[timeline.length - 1]).toMatchObject({ kind: 'message', content: 'Done.' })
  })

  it('keeps mode-change entries in chronological order when hydrated from a snapshot', () => {
    const timeline = buildTimeline({
      ...baseSnapshot,
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
        ...baseSnapshot.turns,
      ],
    })

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

  it('renders plan segments as dedicated timeline entries after hydration', () => {
    const timeline = buildTimeline({
      ...baseSnapshot,
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
    })

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'plan',
      content: '1. Capture the real session path.',
      artifactId: null,
    }))
  })

  it('renders context compaction segments as inline system timeline entries after hydration', () => {
    const timeline = buildTimeline({
      ...baseSnapshot,
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
        ...baseSnapshot.segments,
      ],
    })

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'context_compaction',
      content: 'Context compacted to continue the turn.',
    }))
  })

  it('renders pending request_user_input segments as inline conversation request entries after hydration', () => {
    const timeline = buildTimeline({
      ...baseSnapshot,
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
        ...baseSnapshot.segments,
      ],
    })

    expect(timeline).toContainEqual(expect.objectContaining({
      kind: 'request_user_input',
      content: 'Which path should I take?',
      requestUserInput: expect.objectContaining({
        requestId: 'request-1',
        status: 'pending',
      }),
    }))
  })

  it('preserves expired request_user_input status in hydrated timeline entries', () => {
    const timeline = buildTimeline({
      ...baseSnapshot,
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
        ...baseSnapshot.segments,
      ],
    })

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
