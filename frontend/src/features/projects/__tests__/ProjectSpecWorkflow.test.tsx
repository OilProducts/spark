import { HomeSessionController } from '@/app/AppSessionControllers'
import { ProjectsPanel } from '@/features/projects/ProjectsPanel'
import { type ConversationSnapshotResponse } from '@/lib/apiClient'
import { useStore } from '@/store'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetProjectWorkflowState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'home',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    runRecordOverrides: {},
    workingDir: DEFAULT_WORKING_DIRECTORY,
    model: 'gpt-5.3',
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    logs: [],
  }))
}

const renderProjectsPanel = () =>
  render(
    <>
      <HomeSessionController />
      <ProjectsPanel />
    </>,
  )

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

const cloneSnapshot = (snapshot: ConversationSnapshotResponse): ConversationSnapshotResponse => (
  JSON.parse(JSON.stringify(snapshot)) as ConversationSnapshotResponse
)

const createConversationSnapshot = (
  conversationId: string,
  projectPath: string,
  overrides?: Partial<ConversationSnapshotResponse>,
): ConversationSnapshotResponse => ({
  schema_version: 4,
  conversation_id: conversationId,
  project_path: projectPath,
  title: overrides?.title ?? 'New thread',
  created_at: overrides?.created_at ?? '2026-03-06T15:00:00Z',
  updated_at: overrides?.updated_at ?? overrides?.created_at ?? '2026-03-06T15:00:00Z',
  turns: overrides?.turns ?? [],
  segments: overrides?.segments ?? [],
  event_log: overrides?.event_log ?? [],
  spec_edit_proposals: overrides?.spec_edit_proposals ?? [],
  execution_cards: overrides?.execution_cards ?? [],
  execution_workflow: overrides?.execution_workflow ?? {
    run_id: null,
    status: 'idle',
    error: null,
    flow_source: null,
  },
})

const getConversationIdFromUrl = (url: string): string | null => {
  const match = url.match(/\/api\/conversations\/([^/?]+)/)
  return match ? decodeURIComponent(match[1]) : null
}

class MockEventSource {
  static instances: MockEventSource[] = []

  url: string
  onmessage: ((event: MessageEvent<string>) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  readyState = 1
  closed = false

  constructor(url: string) {
    this.url = url
    MockEventSource.instances.push(this)
  }

  close() {
    this.closed = true
    this.readyState = 2
  }

  emit(payload: unknown) {
    this.onmessage?.({ data: JSON.stringify(payload) } as MessageEvent<string>)
  }
}

describe('Project-scoped workflow behavior', () => {
  let conversationSnapshots: Record<string, ConversationSnapshotResponse>

  beforeEach(() => {
    conversationSnapshots = {}
    MockEventSource.instances = []
    resetProjectWorkflowState()
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        const endpoint = new URL(url, 'http://localhost')
        const conversationId = getConversationIdFromUrl(url)
        const requestBody = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}

        if (endpoint.pathname === '/workspace/api/projects/metadata') {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (endpoint.pathname === '/workspace/api/projects/conversations') {
          const projectPath = endpoint.searchParams.get('project_path') || '/tmp/unknown-project'
          const summaries = Object.values(conversationSnapshots)
            .filter((snapshot) => snapshot.project_path === projectPath)
            .sort((left, right) => right.updated_at.localeCompare(left.updated_at))
            .map((snapshot) => ({
              conversation_id: snapshot.conversation_id,
              project_path: snapshot.project_path,
              title: snapshot.title,
              created_at: snapshot.created_at,
              updated_at: snapshot.updated_at,
              last_message_preview: snapshot.turns
                .filter((turn) => turn.kind === 'message' && turn.content.trim().length > 0)
                .slice(-1)[0]?.content ?? null,
            }))
          return new Response(JSON.stringify(summaries), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (conversationId && endpoint.pathname.endsWith(`/workspace/api/conversations/${conversationId}`) && !init?.method) {
          const projectPath = endpoint.searchParams.get('project_path') || conversationSnapshots[conversationId]?.project_path || '/tmp/unknown-project'
          const snapshot = conversationSnapshots[conversationId]
            ? cloneSnapshot(conversationSnapshots[conversationId]!)
            : createConversationSnapshot(conversationId, projectPath)
          conversationSnapshots[conversationId] = cloneSnapshot(snapshot)
          return new Response(JSON.stringify(snapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (conversationId && endpoint.pathname.endsWith(`/workspace/api/conversations/${conversationId}/turns`) && init?.method === 'POST') {
          const projectPath = String(requestBody.project_path || '/tmp/unknown-project')
          const message = String(requestBody.message || '')
          const proposalId = `proposal-${conversationId}`
          const snapshot = createConversationSnapshot(conversationId, projectPath, {
            turns: [
              {
                id: `turn-user-${conversationId}`,
                role: 'user',
                content: message,
                timestamp: '2026-03-06T15:00:00Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
              {
                id: `turn-assistant-${conversationId}`,
                role: 'assistant',
                content: 'I drafted a spec edit proposal for review.',
                timestamp: '2026-03-06T15:00:10Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
            ],
            segments: [
              {
                id: `segment-tool-${conversationId}`,
                turn_id: `turn-assistant-${conversationId}`,
                order: 1,
                kind: 'tool_call',
                role: 'system',
                status: 'completed',
                timestamp: '2026-03-06T15:00:04Z',
                updated_at: '2026-03-06T15:00:04Z',
                completed_at: '2026-03-06T15:00:04Z',
                content: '',
                artifact_id: null,
                error: null,
                tool_call: {
                  id: `tool-${conversationId}`,
                  kind: 'command_execution',
                  status: 'completed',
                  title: 'Run command',
                  command: 'git status --short',
                  output: ' M frontend/src/features/projects/ProjectsPanel.tsx',
                  file_paths: [],
                },
                source: null,
              },
              {
                id: `segment-proposal-${conversationId}`,
                turn_id: `turn-assistant-${conversationId}`,
                order: 2,
                kind: 'spec_edit_proposal',
                role: 'system',
                status: 'complete',
                timestamp: '2026-03-06T15:00:11Z',
                updated_at: '2026-03-06T15:00:11Z',
                completed_at: '2026-03-06T15:00:11Z',
                content: '',
                artifact_id: proposalId,
                error: null,
                tool_call: null,
                source: null,
              },
              {
                id: `segment-assistant-${conversationId}`,
                turn_id: `turn-assistant-${conversationId}`,
                order: 3,
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
                message: `Drafted spec edit proposal ${proposalId}.`,
                timestamp: '2026-03-06T15:00:11Z',
              },
            ],
            spec_edit_proposals: [
              {
                id: proposalId,
                created_at: '2026-03-06T15:00:11Z',
                summary: 'Project-first home chat flow should require explicit spec review before planning.',
                status: 'pending',
                changes: [
                  {
                    path: 'spec/home-chat.md#review-flow',
                    before: 'Plan generation may begin immediately after the chat turn.',
                    after: 'Plan generation begins only after the user approves a reviewed spec edit card.',
                  },
                ],
                canonical_spec_edit_id: null,
                approved_at: null,
                git_branch: null,
                git_commit: null,
              },
            ],
          })
          conversationSnapshots[conversationId] = cloneSnapshot(snapshot)
          return new Response(JSON.stringify(snapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (conversationId && endpoint.pathname.includes('/spec-edit-proposals/') && endpoint.pathname.endsWith('/approve') && init?.method === 'POST') {
          const currentSnapshot = cloneSnapshot(conversationSnapshots[conversationId]!)
          const proposal = currentSnapshot.spec_edit_proposals[0]!
          proposal.status = 'applied'
          proposal.canonical_spec_edit_id = `spec-edit-${formatProjectKey(currentSnapshot.project_path)}-001`
          proposal.approved_at = '2026-03-06T15:01:00Z'
          proposal.git_branch = 'main'
          proposal.git_commit = 'abc123def456'
          currentSnapshot.event_log = [
            ...currentSnapshot.event_log,
            {
              message: `Approved spec edit proposal ${proposal.id} as canonical spec edit ${proposal.canonical_spec_edit_id} and committed it to git.`,
              timestamp: '2026-03-06T15:01:00Z',
            },
            {
              message: `Execution planning started (workflow-plan-101)${requestBody.flow_source ? ` using ${String(requestBody.flow_source)}.` : '.'}`,
              timestamp: '2026-03-06T15:01:01Z',
            },
          ]
          currentSnapshot.execution_workflow = {
            run_id: 'workflow-plan-101',
            status: 'running',
            error: null,
            flow_source: typeof requestBody.flow_source === 'string' ? requestBody.flow_source : null,
          }
          conversationSnapshots[conversationId] = cloneSnapshot(currentSnapshot)
          return new Response(JSON.stringify(currentSnapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (conversationId && endpoint.pathname.includes('/spec-edit-proposals/') && endpoint.pathname.endsWith('/reject') && init?.method === 'POST') {
          const currentSnapshot = cloneSnapshot(conversationSnapshots[conversationId]!)
          const proposal = currentSnapshot.spec_edit_proposals[0]!
          proposal.status = 'rejected'
          currentSnapshot.event_log = [
            ...currentSnapshot.event_log,
            {
              message: `Rejected spec edit proposal ${proposal.id}.`,
              timestamp: '2026-03-06T15:01:00Z',
            },
          ]
          conversationSnapshots[conversationId] = cloneSnapshot(currentSnapshot)
          return new Response(JSON.stringify(currentSnapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        if (conversationId && endpoint.pathname.includes('/execution-cards/') && endpoint.pathname.endsWith('/review') && init?.method === 'POST') {
          const currentSnapshot = cloneSnapshot(conversationSnapshots[conversationId]!)
          const executionCard = currentSnapshot.execution_cards[currentSnapshot.execution_cards.length - 1]!
          executionCard.status = requestBody.disposition === 'approved'
            ? 'approved'
            : requestBody.disposition === 'rejected'
              ? 'rejected'
              : 'revision-requested'
          executionCard.review_feedback = [
            ...(executionCard.review_feedback || []),
            {
              id: `review-${conversationId}`,
              disposition: String(requestBody.disposition),
              message: String(requestBody.message || ''),
              created_at: '2026-03-06T15:03:00Z',
              author: 'user',
            },
          ]
          currentSnapshot.turns = [
            ...currentSnapshot.turns,
            {
              id: `turn-review-${conversationId}`,
              role: 'user',
              content: String(requestBody.message || ''),
              timestamp: '2026-03-06T15:03:00Z',
              kind: 'message',
              artifact_id: null,
            },
          ]
          conversationSnapshots[conversationId] = cloneSnapshot(currentSnapshot)
          return new Response(JSON.stringify(currentSnapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }

        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  const emitConversationSnapshot = (conversationId: string, snapshot: ConversationSnapshotResponse) => {
    conversationSnapshots[conversationId] = cloneSnapshot(snapshot)
    MockEventSource.instances
      .filter((instance) => !instance.closed && getConversationIdFromUrl(instance.url) === conversationId)
      .forEach((instance) => {
        instance.emit({
          type: 'conversation_snapshot',
          state: snapshot,
        })
      })
  }

  it('creates project-scoped conversation history and requires explicit confirmation before applying proposal edits', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/workflow-project')
      useStore.getState().setActiveProjectPath('/tmp/workflow-project')
    })

    renderProjectsPanel()
    expect(screen.getByTestId('project-ai-conversation-surface')).toBeVisible()

    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Please add a project-first home chat flow with approval before planning.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent(
        'Please add a project-first home chat flow with approval before planning.',
      )
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('git status --short')
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })

    const conversationId = useStore.getState().projectSessionsByPath['/tmp/workflow-project']?.conversationId
    expect(conversationId).toMatch(/^conversation-/)

    const confirmSpy = vi.spyOn(window, 'confirm')
    confirmSpy.mockReturnValueOnce(false)
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))
    expect(useStore.getState().projectSessionsByPath['/tmp/workflow-project']?.specId).toBeNull()

    confirmSpy.mockReturnValueOnce(true)
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))

    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('Applied')
    })

    const projectScope = useStore.getState().projectSessionsByPath['/tmp/workflow-project']
    expect(projectScope.specId).toBe('spec-edit-workflow-project-001')
    expect(projectScope.specStatus).toBe('approved')
    expect(projectScope.projectEventLog).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          message: expect.stringContaining('Approved spec edit proposal'),
        }),
        expect.objectContaining({
          message: expect.stringContaining('Execution planning started'),
        }),
      ]),
    )
    expect(projectScope.specProvenance).toEqual(
      expect.objectContaining({
        source: 'spec-edit-proposal',
        referenceId: `proposal-${conversationId}`,
        runId: null,
        gitBranch: 'main',
        gitCommit: 'abc123def456',
      }),
    )
    expect(screen.queryByTestId('project-spec-edit-proposal-apply-button')).not.toBeInTheDocument()
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('Canonical spec edit')
    expect(screen.getByTestId('project-event-log-list')).toHaveTextContent('Execution planning started')
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Execution planning started')
  })

  it('loads an applied spec edit card from the backend conversation snapshot', async () => {
    const conversationId = 'conversation-persisted-spec-project'
    conversationSnapshots[conversationId] = createConversationSnapshot(conversationId, '/tmp/persisted-spec-project', {
      turns: [
        {
          id: 'turn-user-persisted',
          role: 'user',
          content: 'Refine the home chat and execution cards so tracker-ready work is clearer.',
          timestamp: '2026-03-05T14:00:00Z',
          kind: 'message',
          artifact_id: null,
        },
        {
          id: 'turn-assistant-persisted',
          role: 'assistant',
          content: '',
          timestamp: '2026-03-05T14:02:00Z',
          kind: 'message',
          artifact_id: null,
        },
      ],
      segments: [
        {
          id: 'segment-spec-persisted',
          turn_id: 'turn-assistant-persisted',
          order: 1,
          kind: 'spec_edit_proposal',
          role: 'system',
          status: 'complete',
          timestamp: '2026-03-05T14:02:00Z',
          updated_at: '2026-03-05T14:02:00Z',
          completed_at: '2026-03-05T14:02:00Z',
          content: '',
          artifact_id: 'proposal-persisted-spec-project',
          error: null,
          tool_call: null,
          source: null,
        },
      ],
      spec_edit_proposals: [
        {
          id: 'proposal-persisted-spec-project',
          created_at: '2026-03-05T14:02:00Z',
          summary: 'Refine the home chat and execution cards so tracker-ready work is clearer.',
          status: 'applied',
          changes: [
            {
              path: 'spec/home-chat.md#layout',
              before: 'Home chat mixes workflow state and conversation content.',
              after: 'Home chat shows reviewed artifacts inline while workflow events remain in the event log.',
            },
          ],
          canonical_spec_edit_id: 'spec-edit-persisted-spec-project-001',
          approved_at: '2026-03-05T14:03:00Z',
          git_branch: 'main',
          git_commit: 'abc123def456',
        },
      ],
    })

    act(() => {
      useStore.getState().registerProject('/tmp/persisted-spec-project')
      useStore.getState().setActiveProjectPath('/tmp/persisted-spec-project')
      useStore.getState().setConversationId(conversationId)
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('Applied')
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('proposal-persisted-spec-project')
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('spec-edit-persisted-spec-project-001')
    expect(screen.queryByTestId('project-spec-edit-proposal-apply-button')).not.toBeInTheDocument()
  })

  it('does not render a synthetic spec card for spark without backend artifact state', async () => {
    act(() => {
      useStore.getState().registerProject('/tmp/spark')
      useStore.getState().setActiveProjectPath('/tmp/spark')
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-surface')).toBeVisible()
    })
    expect(screen.queryByTestId('project-spec-edit-proposal-preview')).not.toBeInTheDocument()
    expect(screen.getByTestId('project-ai-conversation-surface')).toHaveTextContent('Create or select a thread to begin chatting.')
  })

  it('adds the execution card from streamed workflow completion and records run context', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/plan-project')
      useStore.getState().setActiveProjectPath('/tmp/plan-project')
      useStore.getState().setActiveFlow('test-planning.dot')
    })

    renderProjectsPanel()
    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Generate a plan for implementing project-home chat proposal approvals.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))

    const conversationId = useStore.getState().projectSessionsByPath['/tmp/plan-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.some((instance) => getConversationIdFromUrl(instance.url) === conversationId)).toBe(true)
    })

    const currentSnapshot = cloneSnapshot(conversationSnapshots[conversationId!])
    const executionSnapshot = createConversationSnapshot(conversationId!, '/tmp/plan-project', {
      turns: [
        ...currentSnapshot.turns,
        {
          id: 'turn-execution-plan-project',
          role: 'assistant',
          content: '',
          timestamp: '2026-03-06T15:02:00Z',
          kind: 'execution_card',
          artifact_id: 'execution-card-plan-project-001',
        },
      ],
      segments: [
        ...currentSnapshot.segments,
        {
          id: 'segment-execution-plan-project',
          turn_id: 'turn-execution-plan-project',
          order: 1,
          kind: 'execution_card',
          role: 'system',
          status: 'complete',
          timestamp: '2026-03-06T15:02:00Z',
          updated_at: '2026-03-06T15:02:00Z',
          completed_at: '2026-03-06T15:02:00Z',
          content: '',
          artifact_id: 'execution-card-plan-project-001',
          error: null,
          tool_call: null,
          source: null,
        },
      ],
      event_log: [
        ...currentSnapshot.event_log,
        {
          message: 'Execution planning completed and produced execution-card-plan-project-001.',
          timestamp: '2026-03-06T15:02:00Z',
        },
      ],
      spec_edit_proposals: currentSnapshot.spec_edit_proposals,
      execution_cards: [
        {
          id: 'execution-card-plan-project-001',
          title: 'Implement project-home chat approval workflow',
          summary: 'Turn the approved spec edit into tracker-ready work.',
          objective: 'Ship reviewed project chat, spec edit approval, and execution-card workflows.',
          status: 'draft',
          source_spec_edit_id: 'spec-edit-plan-project-001',
          source_workflow_run_id: 'workflow-plan-101',
          created_at: '2026-03-06T15:02:00Z',
          updated_at: '2026-03-06T15:02:00Z',
          flow_source: 'test-planning.dot',
          work_items: [
            {
              id: 'WORK-1',
              title: 'Wire project chat to backend conversation snapshots',
              description: 'Replace local stubbed chat state with backend conversation transport and persistence.',
              acceptance_criteria: ['Chat turns render from backend snapshots.'],
              depends_on: [],
            },
          ],
          review_feedback: [],
        },
      ],
      execution_workflow: {
        run_id: 'workflow-plan-101',
        status: 'idle',
        error: null,
        flow_source: 'test-planning.dot',
      },
    })

    act(() => {
      emitConversationSnapshot(conversationId!, executionSnapshot)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-plan-generation-surface')).toBeVisible()
    })

    const state = useStore.getState()
    const scope = state.projectSessionsByPath['/tmp/plan-project']
    expect(scope.planId).toBe('execution-card-plan-project-001')
    expect(scope.planStatus).toBe('draft')
    expect(scope.planProvenance).toEqual(
      expect.objectContaining({
        source: 'execution-card',
        referenceId: 'execution-card-plan-project-001',
        runId: 'workflow-plan-101',
      }),
    )
    expect(screen.getByTestId('project-plan-gate-surface')).toBeVisible()
    expect(screen.getByTestId('project-plan-generation-surface')).toHaveTextContent('WORK-1')
  })

  it('[CID:12.4.03] routes execution-planning status updates to the workflow event log instead of chat history', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/event-log-project')
      useStore.getState().setActiveProjectPath('/tmp/event-log-project')
      useStore.getState().setActiveFlow('test-planning.dot')
    })

    renderProjectsPanel()
    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Draft plan steps and work items for home-first project workflow.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))

    const conversationId = useStore.getState().projectSessionsByPath['/tmp/event-log-project']?.conversationId
    expect(conversationId).toBeTruthy()

    const currentSnapshot = cloneSnapshot(conversationSnapshots[conversationId!])
    const failureSnapshot = createConversationSnapshot(conversationId!, '/tmp/event-log-project', {
      updated_at: '2026-03-06T15:02:30Z',
      turns: currentSnapshot.turns,
      event_log: [
        ...currentSnapshot.event_log,
        {
          message: 'Execution planning failed: app-server session dropped before completion.',
          timestamp: '2026-03-06T15:02:30Z',
        },
      ],
      spec_edit_proposals: currentSnapshot.spec_edit_proposals,
      execution_cards: [],
      execution_workflow: {
        run_id: 'workflow-plan-101',
        status: 'failed',
        error: 'app-server session dropped before completion.',
        flow_source: 'test-planning.dot',
      },
    })

    act(() => {
      emitConversationSnapshot(conversationId!, failureSnapshot)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-event-log-list')).toHaveTextContent('Execution planning failed')
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Execution planning failed')
    expect(screen.queryByTestId('project-plan-generation-surface')).not.toBeInTheDocument()
  })
})

const formatProjectKey = (projectPath: string) => {
  const segments = projectPath.split('/').filter(Boolean)
  return segments[segments.length - 1] || 'project'
}
