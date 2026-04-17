import type { ProjectSessionState } from '@/store'

import { buildProjectsHomeViewModel } from '@/features/projects/model/projectsHomeViewModel'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'

const snapshot: ConversationSnapshotResponse = {
  schema_version: 4,
  conversation_id: 'conversation-1',
  conversation_handle: 'thread-1',
  project_path: '/tmp/project-alpha',
  chat_mode: 'chat',
  title: 'Conversation',
  created_at: '2026-03-24T18:00:00Z',
  updated_at: '2026-03-24T18:01:00Z',
  turns: [
    {
      id: 'turn-user',
      role: 'user',
      content: 'Start planning.',
      timestamp: '2026-03-24T18:00:00Z',
      kind: 'message',
      status: 'complete',
      artifact_id: null,
    },
    {
      id: 'turn-assistant',
      role: 'assistant',
      content: '',
      timestamp: '2026-03-24T18:00:10Z',
      kind: 'message',
      status: 'pending',
      artifact_id: null,
    },
  ],
  segments: [],
  event_log: [],
  flow_run_requests: [
    {
      id: 'request-1',
      status: 'approved',
      created_at: '2026-03-24T18:00:30Z',
      updated_at: '2026-03-24T18:00:31Z',
      flow_name: 'implement-spec.dot',
      summary: 'Launch the implementation flow.',
      project_path: '/tmp/project-alpha',
      conversation_id: 'conversation-1',
      source_turn_id: 'turn-assistant',
      run_id: 'run-7',
      source_segment_id: null,
      goal: 'Launch it',
      launch_context: null,
      launch_error: null,
      review_message: null,
      model: null,
    },
  ],
  flow_launches: [
    {
      id: 'launch-1',
      status: 'launched',
      created_at: '2026-03-24T18:00:40Z',
      updated_at: '2026-03-24T18:00:41Z',
      flow_name: 'implement-spec.dot',
      summary: 'Launch created a run.',
      project_path: '/tmp/project-alpha',
      conversation_id: 'conversation-1',
      source_turn_id: 'turn-assistant',
      run_id: 'run-7',
      source_segment_id: null,
      goal: null,
      launch_context: null,
      model: null,
      launch_error: null,
    },
  ],
}

const activeProjectScope: ProjectSessionState = {
  workingDir: '/tmp/project-alpha',
  conversationId: 'conversation-1',
  projectEventLog: [
    {
      message: 'Conversation resumed.',
      timestamp: '2026-03-24T18:01:00Z',
    },
  ],
}

describe('buildProjectsHomeViewModel', () => {
  it('derives conversation, artifact, and surface state for the home controller', () => {
    const viewModel = buildProjectsHomeViewModel({
      activeConversationId: 'conversation-1',
      activeConversationSnapshot: snapshot,
      activeProjectPath: '/tmp/project-alpha',
      activeProjectScope,
      conversationCache: {
        snapshotsByConversationId: {
          'conversation-1': snapshot,
        },
        summariesByProjectPath: {
          '/tmp/project-alpha': [
            {
              conversation_id: 'conversation-1',
              conversation_handle: 'thread-1',
              project_path: '/tmp/project-alpha',
              title: 'Conversation',
              created_at: '2026-03-24T18:00:00Z',
              updated_at: '2026-03-24T18:01:00Z',
              last_message_preview: 'Start planning.',
            },
          ],
        },
      },
      optimisticSend: {
        conversationId: 'conversation-1',
        createdAt: '2026-03-24T18:01:10Z',
        message: 'Continue',
      },
      projectGitMetadata: {
        '/tmp/project-alpha': {
          branch: 'main',
          commit: 'abc123',
        },
      },
    })

    expect(viewModel.activeProjectLabel).toBe('project-alpha')
    expect(viewModel.activeChatMode).toBe('chat')
    expect(viewModel.activeProjectGitMetadata).toEqual({
      branch: 'main',
      commit: 'abc123',
    })
    expect(viewModel.activeProjectConversationSummaries).toHaveLength(1)
    expect(viewModel.activeProjectEventLog).toEqual(activeProjectScope.projectEventLog)
    expect(viewModel.latestFlowRunRequestId).toBe('request-1')
    expect(viewModel.latestFlowLaunchId).toBe('launch-1')
    expect(viewModel.activeFlowRunRequestsById.get('request-1')?.run_id).toBe('run-7')
    expect(viewModel.activeFlowLaunchesById.get('launch-1')?.status).toBe('launched')
    expect(viewModel.isChatInputDisabled).toBe(true)
    expect(viewModel.chatSendButtonLabel).toBe('Thinking...')
    expect(viewModel.hasRenderableConversationHistory).toBe(true)
    expect(viewModel.activeConversationHistory.at(-1)).toMatchObject({
      role: 'user',
      content: 'Continue',
    })
  })
})
