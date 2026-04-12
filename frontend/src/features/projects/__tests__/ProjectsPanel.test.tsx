import { HomeSessionController } from '@/app/AppSessionControllers'
import { ProjectsPanel } from '@/features/projects/ProjectsPanel'
import { useStore } from '@/store'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const DEFAULT_VIEWPORT_WIDTH = 1280

const setViewportWidth = (width: number) => {
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: width,
  })
  window.dispatchEvent(new Event('resize'))
}

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

const withSnapshotSchema = <T extends Record<string, unknown>>(snapshot: T) => ({
  schema_version: 4,
  ...snapshot,
})

const asSegmentUpsertEvent = (payload: {
  conversation_id: string
  project_path: string
  title: string
  updated_at: string
  event: {
    turn_id: string
    sequence?: number
    timestamp: string
    kind: string
    content_delta?: string
    segment_id?: string
    segment?: Record<string, unknown>
    tool_call?: Record<string, unknown> | null
  }
}) => {
  const segment = payload.event.segment ?? {
    id: payload.event.segment_id ?? `segment-${payload.event.turn_id}-${payload.event.sequence ?? 0}`,
    turn_id: payload.event.turn_id,
    order: payload.event.sequence ?? 0,
    kind:
      payload.event.kind === 'reasoning_summary'
        ? 'reasoning'
        : payload.event.kind.startsWith('tool_call_')
          ? 'tool_call'
          : 'assistant_message',
    role: payload.event.kind.startsWith('tool_call_') ? 'system' : 'assistant',
    status:
      payload.event.kind === 'assistant_completed' || payload.event.kind === 'tool_call_completed'
        ? 'complete'
        : payload.event.kind === 'assistant_failed' || payload.event.kind === 'tool_call_failed'
          ? 'failed'
          : payload.event.kind === 'tool_call_started'
            ? 'running'
            : 'streaming',
    timestamp: payload.event.timestamp,
    updated_at: payload.updated_at,
    completed_at:
      payload.event.kind === 'assistant_completed' || payload.event.kind === 'tool_call_completed'
        ? payload.updated_at
        : null,
    content: payload.event.content_delta ?? '',
    artifact_id: null,
    error: null,
    tool_call: payload.event.tool_call ?? null,
    source: null,
  }

  return {
    type: 'segment_upsert' as const,
    conversation_id: payload.conversation_id,
    project_path: payload.project_path,
    title: payload.title,
    updated_at: payload.updated_at,
    segment,
  }
}

const resetProjectScopeState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    homeConversationCache: {
      snapshotsByConversationId: {},
      summariesByProjectPath: {},
    },
    homeThreadSummariesStatusByProjectPath: {},
    homeThreadSummariesErrorByProjectPath: {},
    homeProjectSessionsByPath: {},
    homeConversationSessionsById: {},
    homeProjectGitMetadataByPath: {},
  }))
}

const renderProjectsPanel = () =>
  render(
    <>
      <HomeSessionController />
      <ProjectsPanel />
    </>,
  )

const renderProjectsPanelWithoutHomeController = () => render(<ProjectsPanel />)

describe('ProjectsPanel', () => {
  beforeEach(() => {
    setViewportWidth(DEFAULT_VIEWPORT_WIDTH)
    resetProjectScopeState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
      if (url.includes('/workspace/api/projects/pick-directory')) {
        return new Response(JSON.stringify({ status: 'canceled' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/workspace/api/projects/metadata')) {
        return new Response(JSON.stringify({ branch: 'main', commit: 'abcdef0' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/workspace/api/projects/conversations')) {
        return new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/workspace/api/projects/register')) {
        return new Response(
          JSON.stringify({
            project_id: 'project-quick-switch',
            project_path: '/tmp/quick-switch-project',
            display_name: 'quick-switch-project',
            created_at: new Date().toISOString(),
            last_opened_at: new Date().toISOString(),
            last_accessed_at: null,
            is_favorite: false,
            active_conversation_id: null,
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          },
        )
      }
      if (url.includes('/workspace/api/projects')) {
        return new Response(
          JSON.stringify([
            {
              project_id: 'project-quick-switch',
              project_path: '/tmp/quick-switch-project',
              display_name: 'quick-switch-project',
              created_at: new Date().toISOString(),
              last_opened_at: new Date().toISOString(),
              last_accessed_at: null,
              is_favorite: false,
              active_conversation_id: null,
            },
          ]),
          {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          },
        )
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

  it('renders a threads-first sidebar and event log', async () => {
    renderProjectsPanel()

    expect(screen.getByText('Threads')).toBeVisible()
    expect(screen.getByTestId('project-thread-list')).toBeVisible()
    expect(screen.getByTestId('project-event-log-surface')).toBeVisible()
    expect(
      within(screen.getByTestId('project-thread-list')).getByText(
        'Choose or add a project from the navbar to view threads.',
      ),
    ).toBeVisible()

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/quick-switch-project']).toBeDefined()
    })
  })

  it('uses a full-height split shell on wide viewports', () => {
    useStore.setState((state) => ({
      ...state,
      projectRegistry: {
        '/tmp/existing-project': {
          project_id: 'existing-project',
          project_path: '/tmp/existing-project',
          display_name: 'existing-project',
          created_at: new Date().toISOString(),
          last_opened_at: new Date().toISOString(),
          last_accessed_at: null,
          is_favorite: false,
          active_conversation_id: null,
        },
      },
    }))

    renderProjectsPanelWithoutHomeController()

    const projectsPanel = screen.getByTestId('projects-panel')
    const homeMainLayout = screen.getByTestId('home-main-layout')

    expect(projectsPanel).toHaveAttribute('data-responsive-layout', 'split')
    expect(projectsPanel).toHaveClass('h-full')
    expect(homeMainLayout).toHaveClass('flex-1')
    expect(homeMainLayout).toHaveClass('min-h-0')
  })

  it('surfaces a project registry bootstrap error on refresh', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects') && !url.includes('/workspace/api/projects/conversations') && !url.includes('/workspace/api/projects/metadata') && !url.includes('/workspace/api/projects/register') && !url.includes('/workspace/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ detail: 'workspace registry unavailable' }), {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ status: 'canceled' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abcdef0' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
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

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-panel-error')).toHaveTextContent('workspace registry unavailable')
    })
    expect(useStore.getState().projectRegistry).toEqual({})
  })

  it('lets the operator resize sidebar sections in desktop layout', async () => {
    act(() => {
      useStore.getState().registerProject('/tmp/quick-switch-project')
      useStore.getState().setActiveProjectPath('/tmp/quick-switch-project')
    })

    renderProjectsPanelWithoutHomeController()

    const sidebarStack = screen.getByTestId('home-sidebar-stack')
    const sidebarPrimarySurface = screen.getByTestId('home-sidebar-primary-surface') as HTMLDivElement
    const resizeHandle = screen.getByTestId('home-sidebar-resize-handle')

    vi.spyOn(sidebarStack, 'getBoundingClientRect').mockReturnValue({
      x: 0,
      y: 0,
      top: 0,
      right: 320,
      bottom: 720,
      left: 0,
      width: 320,
      height: 720,
      toJSON: () => ({}),
    } as DOMRect)

    expect(sidebarPrimarySurface.style.height).toBe('320px')

    fireEvent.pointerDown(resizeHandle, { clientY: 240 })
    fireEvent.pointerMove(window, { clientY: 300 })
    fireEvent.pointerUp(window)

    expect(sidebarPrimarySurface.style.height).toBe('380px')
  })

  it('points empty states at the navbar project switcher when no project is active', async () => {
    renderProjectsPanelWithoutHomeController()

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/quick-switch-project']).toBeDefined()
    })

    expect(
      within(screen.getByTestId('project-thread-list')).getByText(
        'Choose or add a project from the navbar to view threads.',
      ),
    ).toBeVisible()
    expect(
      within(screen.getByTestId('project-event-log-surface')).getByText(
        'Choose or add a project from the navbar to view workflow events.',
      ),
    ).toBeVisible()
    expect(
      within(screen.getByTestId('project-ai-conversation-surface')).getByText(
        'Choose or add a project from the navbar to begin chatting.',
      ),
    ).toBeVisible()
  })

  it('shows thread controls for the active project', async () => {
    act(() => {
      useStore.getState().registerProject('/tmp/quick-switch-project')
      useStore.getState().setActiveProjectPath('/tmp/quick-switch-project')
    })

    renderProjectsPanel()

    expect(screen.getByTestId('project-thread-new-button')).toBeVisible()
    expect(screen.getByText('Threads for quick-switch-project.')).toBeVisible()
    await waitFor(() => {
      expect(screen.getByText('No threads for this project yet.')).toBeVisible()
    })
  })

  it('keeps the workflow event log project-scoped', async () => {
    act(() => {
      useStore.getState().registerProject('/tmp/quick-switch-project')
      useStore.getState().setActiveProjectPath('/tmp/quick-switch-project')
      useStore.getState().appendProjectEventEntry({
        message: 'Approved plan',
        timestamp: '2026-03-24T12:00:00Z',
      })
    })

    renderProjectsPanelWithoutHomeController()

    expect(screen.getByTestId('project-event-log-list')).toHaveTextContent('Approved plan')
  })

  it('renders the user turn before the assistant response completes', async () => {
    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Show this message immediately.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Show this message immediately.')
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Thinking...')
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: 'conversation-chat-project-1',
          project_path: '/tmp/chat-project',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Show this message immediately.',
              timestamp: '2026-03-06T21:45:00Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'Visible.',
              timestamp: '2026-03-06T21:45:02Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
          ],
          turn_events: [
            {
              id: 'event-assistant-delta-1',
              turn_id: 'turn-assistant-1',
              sequence: 1,
              timestamp: '2026-03-06T21:45:01Z',
              kind: 'assistant_delta',
              content_delta: 'Visible.',
            },
            {
              id: 'event-assistant-complete-1',
              turn_id: 'turn-assistant-1',
              sequence: 2,
              timestamp: '2026-03-06T21:45:02Z',
              kind: 'assistant_completed',
              message: 'Assistant turn completed.',
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Visible.')
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Worked for')
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')
  })

  it('renders the assistant reply even when the backend canonicalizes project_path', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return new Response(
            JSON.stringify(withSnapshotSchema({
              conversation_id: 'conversation-home-project-1',
              project_path: '/System/Volumes/Data/home/chris/tinker/spark',
              title: 'Reply with a one-line acknowledgement only.',
              created_at: '2026-03-07T15:55:00Z',
              updated_at: '2026-03-07T15:55:04Z',
              turns: [
                {
                  id: 'turn-user-1',
                  role: 'user',
                  content: 'Reply with a one-line acknowledgement only.',
                  timestamp: '2026-03-07T15:55:00Z',
                  kind: 'message',
                  artifact_id: null,
                },
                {
                  id: 'turn-assistant-1',
                  role: 'assistant',
                  content: 'Acknowledged.',
                  timestamp: '2026-03-07T15:55:04Z',
                  kind: 'message',
                  artifact_id: null,
                },
              ],
              event_log: [],



            })),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/home/chris/tinker/spark')
    useStore.getState().setActiveProjectPath('/home/chris/tinker/spark')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Reply with a one-line acknowledgement only.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Acknowledged.')
    })
  })

  it('disables sending while an assistant turn is still active in the conversation snapshot', async () => {
    const user = userEvent.setup()
    const sendRequests: string[] = []

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([
            {
              conversation_id: 'conversation-active-turn',
              conversation_handle: 'amber-otter',
              project_path: '/tmp/chat-project',
              title: 'Active thread',
              created_at: '2026-03-15T14:05:00Z',
              updated_at: '2026-03-15T14:05:02Z',
              last_message_preview: 'Still working on it.',
            },
          ]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/conversation-active-turn') && !url.includes('/turns')) {
          return new Response(JSON.stringify(withSnapshotSchema({
            conversation_id: 'conversation-active-turn',
            conversation_handle: 'amber-otter',
            project_path: '/tmp/chat-project',
            title: 'Active thread',
            created_at: '2026-03-15T14:05:00Z',
            updated_at: '2026-03-15T14:05:02Z',
            turns: [
              {
                id: 'turn-user-1',
                role: 'user',
                content: 'Keep going.',
                timestamp: '2026-03-15T14:05:00Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
              {
                id: 'turn-assistant-1',
                role: 'assistant',
                content: '',
                timestamp: '2026-03-15T14:05:01Z',
                status: 'streaming',
                kind: 'message',
                artifact_id: null,
                parent_turn_id: 'turn-user-1',
              },
            ],
            segments: [
              {
                id: 'segment-reasoning-1',
                turn_id: 'turn-assistant-1',
                order: 1,
                kind: 'reasoning',
                role: 'assistant',
                status: 'streaming',
                timestamp: '2026-03-15T14:05:02Z',
                updated_at: '2026-03-15T14:05:02Z',
                completed_at: null,
                content: 'Still working on it.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: null,
              },
            ],
            event_log: [],



          })), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          sendRequests.push(url)
          return new Response(JSON.stringify({ detail: 'should not send while assistant turn is active' }), {
            status: 500,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Still working on it.')
    })

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Can I interrupt?')

    const sendButton = screen.getByTestId('project-ai-conversation-send-button')
    expect(sendButton).toBeDisabled()
    expect(sendButton).toHaveTextContent('Thinking...')

    await user.click(sendButton)

    expect(sendRequests).toHaveLength(0)
  })

  it('renders assistant tool calls before the completed assistant summary for the same turn', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(
            JSON.stringify(withSnapshotSchema({
              conversation_id: 'conversation-ordering-1',
              project_path: '/tmp/chat-project',
              title: 'Ordering thread',
              created_at: '2026-03-07T19:45:43Z',
              updated_at: '2026-03-07T19:46:30Z',
              turns: [
                {
                  id: 'turn-user-1',
                  role: 'user',
                  content: 'Run ls then ps and summarize them.',
                  timestamp: '2026-03-07T19:46:00Z',
                  status: 'complete',
                  kind: 'message',
                  artifact_id: null,
                },
                {
                  id: 'turn-assistant-1',
                  role: 'assistant',
                  content: 'Summary after tools.',
                  timestamp: '2026-03-07T19:46:30Z',
                  status: 'complete',
                  kind: 'message',
                  artifact_id: null,
                  parent_turn_id: 'turn-user-1',
                },
              ],
              segments: [
                {
                  id: 'segment-tool-ls',
                  turn_id: 'turn-assistant-1',
                  order: 1,
                  kind: 'tool_call',
                  role: 'system',
                  status: 'completed',
                  timestamp: '2026-03-07T19:46:10Z',
                  updated_at: '2026-03-07T19:46:12Z',
                  completed_at: '2026-03-07T19:46:12Z',
                  content: '',
                  artifact_id: null,
                  error: null,
                  tool_call: {
                    id: 'tool-ls',
                    kind: 'command_execution',
                    status: 'completed',
                    title: 'Run command',
                    command: '/bin/zsh -lc ls',
                    output: 'AGENTS.md',
                    file_paths: [],
                  },
                  source: null,
                },
                {
                  id: 'segment-tool-ps',
                  turn_id: 'turn-assistant-1',
                  order: 2,
                  kind: 'tool_call',
                  role: 'system',
                  status: 'completed',
                  timestamp: '2026-03-07T19:46:20Z',
                  updated_at: '2026-03-07T19:46:22Z',
                  completed_at: '2026-03-07T19:46:22Z',
                  content: '',
                  artifact_id: null,
                  error: null,
                  tool_call: {
                    id: 'tool-ps',
                    kind: 'command_execution',
                    status: 'completed',
                    title: 'Run command',
                    command: '/bin/zsh -lc ps',
                    output: 'PID TTY TIME CMD',
                    file_paths: [],
                  },
                  source: null,
                },
                {
                  id: 'segment-assistant-summary',
                  turn_id: 'turn-assistant-1',
                  order: 3,
                  kind: 'assistant_message',
                  role: 'assistant',
                  status: 'complete',
                  timestamp: '2026-03-07T19:46:30Z',
                  updated_at: '2026-03-07T19:46:30Z',
                  completed_at: '2026-03-07T19:46:30Z',
                  content: 'Summary after tools.',
                  artifact_id: null,
                  error: null,
                  tool_call: null,
                  source: null,
                },
              ],
              event_log: [],



            })),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({ detail: 'not found' }), {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')
    useStore.getState().setConversationId('conversation-ordering-1')

    renderProjectsPanel()

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    const text = history.textContent ?? ''

    expect(text.indexOf('/bin/zsh -lc ls')).toBeGreaterThan(-1)
    expect(text.indexOf('/bin/zsh -lc ps')).toBeGreaterThan(-1)
    expect(text.indexOf('Worked for 20s')).toBeGreaterThan(-1)
    expect(text.indexOf('Summary after tools.')).toBeGreaterThan(-1)
    expect(text.indexOf('/bin/zsh -lc ls')).toBeLessThan(text.indexOf('Summary after tools.'))
    expect(text.indexOf('/bin/zsh -lc ps')).toBeLessThan(text.indexOf('Summary after tools.'))
    expect(text.indexOf('Worked for 20s')).toBeLessThan(text.indexOf('Summary after tools.'))
  })

  it('renders tool calls collapsed by default and expands them on demand', async () => {
    const user = userEvent.setup()

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(
            JSON.stringify(withSnapshotSchema({
              conversation_id: 'conversation-tool-collapse-1',
              project_path: '/tmp/chat-project',
              title: 'Collapsed tool call thread',
              created_at: '2026-03-07T19:45:43Z',
              updated_at: '2026-03-07T19:46:30Z',
              turns: [
                {
                  id: 'turn-user-1',
                  role: 'user',
                  content: 'Run ls and summarize it.',
                  timestamp: '2026-03-07T19:46:00Z',
                  status: 'complete',
                  kind: 'message',
                  artifact_id: null,
                },
                {
                  id: 'turn-assistant-1',
                  role: 'assistant',
                  content: 'Summary after tools.',
                  timestamp: '2026-03-07T19:46:30Z',
                  status: 'complete',
                  kind: 'message',
                  artifact_id: null,
                  parent_turn_id: 'turn-user-1',
                },
              ],
              segments: [
                {
                  id: 'segment-tool-ls',
                  turn_id: 'turn-assistant-1',
                  order: 1,
                  kind: 'tool_call',
                  role: 'system',
                  status: 'completed',
                  timestamp: '2026-03-07T19:46:12Z',
                  updated_at: '2026-03-07T19:46:12Z',
                  completed_at: '2026-03-07T19:46:12Z',
                  content: '',
                  artifact_id: null,
                  error: null,
                  tool_call: {
                    id: 'tool-ls',
                    kind: 'command_execution',
                    status: 'completed',
                    title: 'Run command',
                    command: '/bin/zsh -lc ls',
                    output: 'AGENTS.md\nREADME.md',
                    file_paths: [],
                  },
                  source: null,
                },
                {
                  id: 'segment-assistant-summary',
                  turn_id: 'turn-assistant-1',
                  order: 2,
                  kind: 'assistant_message',
                  role: 'assistant',
                  status: 'complete',
                  timestamp: '2026-03-07T19:46:30Z',
                  updated_at: '2026-03-07T19:46:30Z',
                  completed_at: '2026-03-07T19:46:30Z',
                  content: 'Summary after tools.',
                  artifact_id: null,
                  error: null,
                  tool_call: null,
                  source: null,
                },
              ],
              event_log: [],



            })),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')
    useStore.getState().setConversationId('conversation-tool-collapse-1')

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('/bin/zsh -lc ls')
    })
    expect(screen.queryByText(/AGENTS\.md/)).not.toBeInTheDocument()

    await user.click(screen.getByTestId('project-tool-call-toggle-tool-ls'))

    expect(screen.getByText(/AGENTS\.md/)).toBeInTheDocument()
    expect(screen.getByText(/README\.md/)).toBeInTheDocument()
  })

  it('renders thinking summaries collapsed to their bold lead-in and expands the details on demand', async () => {
    const user = userEvent.setup()

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(
            JSON.stringify(withSnapshotSchema({
              conversation_id: 'conversation-thinking-collapse-1',
              project_path: '/tmp/chat-project',
              title: 'Collapsed thinking thread',
              created_at: '2026-03-07T20:10:00Z',
              updated_at: '2026-03-07T20:10:04Z',
              turns: [
                {
                  id: 'turn-user-1',
                  role: 'user',
                  content: 'What are you doing?',
                  timestamp: '2026-03-07T20:10:00Z',
                  status: 'complete',
                  kind: 'message',
                  artifact_id: null,
                },
                {
                  id: 'turn-assistant-1',
                  role: 'assistant',
                  content: '',
                  timestamp: '2026-03-07T20:10:01Z',
                  status: 'streaming',
                  kind: 'message',
                  artifact_id: null,
                  parent_turn_id: 'turn-user-1',
                },
              ],
              segments: [
                {
                  id: 'segment-reasoning-1',
                  turn_id: 'turn-assistant-1',
                  order: 1,
                  kind: 'reasoning',
                  role: 'assistant',
                  status: 'streaming',
                  timestamp: '2026-03-07T20:10:02Z',
                  updated_at: '2026-03-07T20:10:02Z',
                  completed_at: null,
                  content: '**Considering proposal** Looking for the smallest safe change first.',
                  artifact_id: null,
                  error: null,
                  tool_call: null,
                  source: null,
                },
              ],
              event_log: [],



            })),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')
    useStore.getState().setConversationId('conversation-thinking-collapse-1')

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Considering proposal')
    })
    expect(screen.queryByText('Looking for the smallest safe change first.')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('project-thinking-toggle-segment-reasoning-1'))

    expect(screen.getByText('Looking for the smallest safe change first.')).toBeInTheDocument()
  })

  it('streams assistant text into the history before the turn response completes', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Stream this reply.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })
    expect(MockEventSource.instances).toHaveLength(1)

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:02Z',
          turn: {
            id: 'turn-assistant-live',
            role: 'assistant',
            content: 'Working on',
            timestamp: '2026-03-07T15:30:02Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:02Z',
          event: {
            id: 'event-assistant-delta-live',
            turn_id: 'turn-assistant-live',
            sequence: 1,
            timestamp: '2026-03-07T15:30:02Z',
            kind: 'assistant_delta',
            content_delta: 'Working on',
          },
        })),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Working on')
    })
    expect(MockEventSource.instances).toHaveLength(1)
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Thinking...')

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          created_at: '2026-03-07T15:30:00Z',
          updated_at: '2026-03-07T15:30:04Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Stream this reply.',
              timestamp: '2026-03-07T15:30:00Z',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'Working on it.',
              timestamp: '2026-03-07T15:30:04Z',
              kind: 'message',
              artifact_id: null,
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Working on it.')
    })
    expect(MockEventSource.instances).toHaveLength(1)
    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')
    })
  })

  it('ignores a stale send-response snapshot after the stream has already advanced the turn', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Keep the streamed thinking visible.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Keep the streamed thinking visible.',
          updated_at: '2026-03-07T15:31:02Z',
          turn: {
            id: 'turn-assistant-live',
            role: 'assistant',
            content: '',
            timestamp: '2026-03-07T15:31:02Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-1',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Keep the streamed thinking visible.',
          updated_at: '2026-03-07T15:31:02Z',
          event: {
            id: 'event-reasoning-live',
            turn_id: 'turn-assistant-live',
            sequence: 1,
            timestamp: '2026-03-07T15:31:02Z',
            kind: 'reasoning_summary',
            content_delta: 'Scanning the project layout first.',
            segment_id: 'segment-reasoning-live',
            segment: {
              id: 'segment-reasoning-live',
              turn_id: 'turn-assistant-live',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-07T15:31:02Z',
              updated_at: '2026-03-07T15:31:02Z',
              completed_at: null,
              content: 'Scanning the project layout first.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Scanning the project layout first.')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Keep the streamed thinking visible.',
          created_at: '2026-03-07T15:31:00Z',
          updated_at: '2026-03-07T15:31:02Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Keep the streamed thinking visible.',
              timestamp: '2026-03-07T15:31:00Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-live',
              role: 'assistant',
              content: '',
              timestamp: '2026-03-07T15:31:01Z',
              status: 'pending',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          segments: [
            {
              id: 'segment-reasoning-live',
              turn_id: 'turn-assistant-live',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-07T15:31:02Z',
              updated_at: '2026-03-07T15:31:02Z',
              completed_at: null,
              content: 'Scanning the project layout first.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Scanning the project layout first.')
    })
  })

  it('renders flow run request cards, approves them, and opens the launched run', async () => {
    const user = userEvent.setup()
    const reviewCalls: Array<{ url: string; body: Record<string, unknown> }> = []

    const pendingSnapshot = withSnapshotSchema({
      conversation_id: 'conversation-flow-run',
      conversation_handle: 'amber-otter',
      project_path: '/tmp/chat-project',
      title: 'Run implementation',
      created_at: '2026-03-09T21:00:00Z',
      updated_at: '2026-03-09T21:00:03Z',
      turns: [
        {
          id: 'turn-user-flow-run',
          role: 'user',
          content: 'Kick off implementation.',
          timestamp: '2026-03-09T21:00:00Z',
          status: 'complete',
          kind: 'message',
          artifact_id: null,
        },
        {
          id: 'turn-assistant-flow-run',
          role: 'assistant',
          content: 'I can request that launch.',
          timestamp: '2026-03-09T21:00:01Z',
          status: 'complete',
          kind: 'message',
          artifact_id: null,
        },
      ],
      segments: [
        {
          id: 'segment-artifact-flow-run-request-inline',
          turn_id: 'turn-assistant-flow-run',
          order: 1,
          kind: 'flow_run_request',
          role: 'system',
          status: 'complete',
          timestamp: '2026-03-09T21:00:02Z',
          updated_at: '2026-03-09T21:00:02Z',
          completed_at: '2026-03-09T21:00:02Z',
          content: '',
          artifact_id: 'flow-run-request-inline',
          error: null,
          tool_call: null,
          source: null,
        },
      ],
      event_log: [],

      flow_run_requests: [
        {
          id: 'flow-run-request-inline',
          created_at: '2026-03-09T21:00:02Z',
          updated_at: '2026-03-09T21:00:02Z',
          flow_name: 'test-dispatch.dot',
          summary: 'Run implementation for the approved scope.',
          project_path: '/tmp/chat-project',
          conversation_id: 'conversation-flow-run',
          source_turn_id: 'turn-assistant-flow-run',
          source_segment_id: 'segment-artifact-flow-run-request-inline',
          status: 'pending',
          goal: 'Implement the approved scope.',
          model: 'gpt-5.4',
          run_id: null,
          launch_error: null,
          review_message: null,
        },
      ],


    })

    const launchedSnapshot = withSnapshotSchema({
      ...pendingSnapshot,
      updated_at: '2026-03-09T21:00:05Z',
      flow_run_requests: [
        {
          ...pendingSnapshot.flow_run_requests[0],
          updated_at: '2026-03-09T21:00:05Z',
          status: 'launched',
          run_id: 'run-flow-123',
          review_message: 'Approved for launch.',
        },
      ],
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([
            {
              conversation_id: 'conversation-flow-run',
              conversation_handle: 'amber-otter',
              project_path: '/tmp/chat-project',
              title: 'Run implementation',
              created_at: '2026-03-09T21:00:00Z',
              updated_at: '2026-03-09T21:00:03Z',
              last_message_preview: 'I can request that launch.',
            },
          ]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/conversation-flow-run/flow-run-requests/flow-run-request-inline/review')) {
          reviewCalls.push({
            url,
            body: JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>,
          })
          return new Response(JSON.stringify(launchedSnapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/conversation-flow-run') && !init?.method) {
          return new Response(JSON.stringify(pendingSnapshot), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response('Not found', { status: 404 })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')
    useStore.getState().updateProjectSessionState('/tmp/chat-project', {
      conversationId: 'conversation-flow-run',
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-flow-run-request-approve-button')).toBeVisible()
    })

    await user.click(screen.getByTestId('project-flow-run-request-approve-button'))

    await waitFor(() => {
      expect(reviewCalls).toHaveLength(1)
      expect(screen.getByTestId('project-flow-run-request-open-run-button')).toBeVisible()
    })

    expect(reviewCalls[0]?.body).toMatchObject({
      project_path: '/tmp/chat-project',
      disposition: 'approved',
      message: 'Approved for launch.',
    })

    await user.click(screen.getByTestId('project-flow-run-request-open-run-button'))

    await waitFor(() => {
      expect(useStore.getState().selectedRunId).toBe('run-flow-123')
      expect(useStore.getState().executionFlow).toBe('test-dispatch.dot')
      expect(useStore.getState().viewMode).toBe('runs')
    })
  })

  it('returns the send button to Send once the assistant turn completes even if the original POST is still pending', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Stream this reply.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:02Z',
          turn: {
            id: 'turn-assistant-live',
            role: 'assistant',
            content: '',
            timestamp: '2026-03-07T15:30:02Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:02Z',
          event: {
            id: 'event-assistant-delta-live',
            turn_id: 'turn-assistant-live',
            sequence: 1,
            timestamp: '2026-03-07T15:30:02Z',
            kind: 'assistant_delta',
            content_delta: 'Working on',
          },
        })),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Thinking...')
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:04Z',
          event: {
            id: 'event-assistant-complete-live',
            turn_id: 'turn-assistant-live',
            sequence: 2,
            timestamp: '2026-03-07T15:30:04Z',
            kind: 'assistant_completed',
            content_delta: 'Working on it.',
            segment: {
              id: 'segment-assistant-live',
              turn_id: 'turn-assistant-live',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-07T15:30:04Z',
              updated_at: '2026-03-07T15:30:04Z',
              completed_at: '2026-03-07T15:30:04Z',
              content: 'Working on it.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
              phase: 'final_answer',
            },
          },
        })),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          updated_at: '2026-03-07T15:30:04Z',
          turn: {
            id: 'turn-assistant-live',
            role: 'assistant',
            content: 'Working on it.',
            timestamp: '2026-03-07T15:30:02Z',
            status: 'complete',
            kind: 'message',
            artifact_id: null,
          },
        }),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Working on it.')
      expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Stream this reply.',
          created_at: '2026-03-07T15:30:00Z',
          updated_at: '2026-03-07T15:30:04Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Stream this reply.',
              timestamp: '2026-03-07T15:30:00Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-live',
              role: 'assistant',
              content: 'Working on it.',
              timestamp: '2026-03-07T15:30:02Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
          ],
          segments: [
            {
              id: 'segment-assistant-live',
              turn_id: 'turn-assistant-live',
              order: 1,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-07T15:30:04Z',
              updated_at: '2026-03-07T15:30:04Z',
              completed_at: '2026-03-07T15:30:04Z',
              content: 'Working on it.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
              phase: 'final_answer',
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')
    })
  })

  it('preserves reasoning, tool calls, and post-tool assistant text ordering when the final snapshot compacts transient events', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Draft a spec.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          updated_at: '2026-03-08T19:10:01Z',
          turn: {
            id: 'turn-assistant-1',
            role: 'assistant',
            content: 'I’m going to scan the repository structure first.',
            timestamp: '2026-03-08T19:10:01Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-1',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          updated_at: '2026-03-08T19:10:00Z',
          event: {
            id: 'event-reasoning-1',
            turn_id: 'turn-assistant-1',
            sequence: 0,
            timestamp: '2026-03-08T19:10:00Z',
            kind: 'reasoning_summary',
            content_delta: 'Scanning the repository structure first.',
            segment_id: 'segment-reasoning-1',
            segment: {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:10:00Z',
              updated_at: '2026-03-08T19:10:00Z',
              completed_at: null,
              content: 'Scanning the repository structure first.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          updated_at: '2026-03-08T19:10:01Z',
          event: {
            id: 'event-assistant-delta-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T19:10:01Z',
            kind: 'assistant_delta',
            content_delta: 'I’m going to scan the repository structure first.',
            segment_id: 'segment-assistant-1',
            segment: {
              id: 'segment-assistant-1',
              turn_id: 'turn-assistant-1',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:10:01Z',
              updated_at: '2026-03-08T19:10:01Z',
              completed_at: null,
              content: 'I’m going to scan the repository structure first.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          updated_at: '2026-03-08T19:10:02Z',
          event: {
            id: 'event-tool-started-1',
            turn_id: 'turn-assistant-1',
            sequence: 2,
            timestamp: '2026-03-08T19:10:02Z',
            kind: 'tool_call_started',
            tool_call_id: 'tool-ls',
            tool_call: {
              id: 'tool-ls',
              kind: 'command_execution',
              status: 'running',
              title: 'Run command',
              command: '/bin/zsh -lc ls',
              output: null,
              file_paths: [],
            },
            segment_id: 'segment-tool-ls',
            segment: {
              id: 'segment-tool-ls',
              turn_id: 'turn-assistant-1',
              order: 3,
              kind: 'tool_call',
              role: 'system',
              status: 'running',
              timestamp: '2026-03-08T19:10:02Z',
              updated_at: '2026-03-08T19:10:02Z',
              completed_at: null,
              content: '',
              artifact_id: null,
              error: null,
              tool_call: {
                id: 'tool-ls',
                kind: 'command_execution',
                status: 'running',
                title: 'Run command',
                command: '/bin/zsh -lc ls',
                output: null,
                file_paths: [],
              },
              source: null,
            },
          },
        })),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          updated_at: '2026-03-08T19:10:03Z',
          event: {
            id: 'event-assistant-delta-2',
            turn_id: 'turn-assistant-1',
            sequence: 3,
            timestamp: '2026-03-08T19:10:03Z',
            kind: 'assistant_delta',
            content_delta: 'I found the main entry points and can summarize them.',
            segment_id: 'segment-assistant-2',
            segment: {
              id: 'segment-assistant-2',
              turn_id: 'turn-assistant-1',
              order: 4,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:10:03Z',
              updated_at: '2026-03-08T19:10:03Z',
              completed_at: null,
              content: 'I found the main entry points and can summarize them.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Scanning the repository structure first.')
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('I found the main entry points and can summarize them.')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Draft a spec.',
          created_at: '2026-03-08T19:10:00Z',
          updated_at: '2026-03-08T19:10:04Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Draft a spec.',
              timestamp: '2026-03-08T19:10:00Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'I’m going to scan the repository structure first. I found the main entry points and can summarize them.',
              timestamp: '2026-03-08T19:10:01Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          segments: [
            {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:10:00Z',
              updated_at: '2026-03-08T19:10:00Z',
              completed_at: null,
              content: 'Scanning the repository structure first.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
            {
              id: 'segment-tool-ls',
              turn_id: 'turn-assistant-1',
              order: 2,
              kind: 'tool_call',
              role: 'system',
              status: 'completed',
              timestamp: '2026-03-08T19:10:02Z',
              updated_at: '2026-03-08T19:10:02Z',
              completed_at: '2026-03-08T19:10:02Z',
              content: '',
              artifact_id: null,
              error: null,
              tool_call: {
                id: 'tool-ls',
                kind: 'command_execution',
                status: 'completed',
                title: 'Run command',
                command: '/bin/zsh -lc ls',
                output: 'README.md',
                file_paths: [],
              },
              source: null,
            },
            {
              id: 'segment-assistant-2',
              turn_id: 'turn-assistant-1',
              order: 3,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-08T19:10:03Z',
              updated_at: '2026-03-08T19:10:04Z',
              completed_at: '2026-03-08T19:10:04Z',
              content: 'I found the main entry points and can summarize them.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
          const text = history.textContent ?? ''
      expect(text.indexOf('Scanning the repository structure first.')).toBeGreaterThan(-1)
      expect(text.indexOf('/bin/zsh -lc ls')).toBeGreaterThan(-1)
      expect(text.indexOf('I found the main entry points and can summarize them.')).toBeGreaterThan(-1)
      expect(text.indexOf('Scanning the repository structure first.')).toBeLessThan(text.indexOf('/bin/zsh -lc ls'))
      expect(text.indexOf('/bin/zsh -lc ls')).toBeLessThan(text.indexOf('I found the main entry points and can summarize them.'))
    })
  })

  it('orders preserved transient assistant events ahead of compacted completion events when sequences collide', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Can you use the flow run request too?')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          updated_at: '2026-03-08T19:28:40Z',
          turn: {
            id: 'turn-assistant-1',
            role: 'assistant',
            content: 'Once you give me a flow and goal, I can create a pending flow run request.',
            timestamp: '2026-03-08T19:28:40Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-1',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          updated_at: '2026-03-08T19:28:38Z',
          event: {
            id: 'event-reasoning-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T19:28:38Z',
            kind: 'reasoning_summary',
            content_delta: 'Considering the flow run request.',
            segment_id: 'segment-reasoning-1',
            segment: {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:28:38Z',
              updated_at: '2026-03-08T19:28:38Z',
              completed_at: null,
              content: 'Considering the flow run request.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          updated_at: '2026-03-08T19:28:40Z',
          event: {
            id: 'event-assistant-delta-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T19:28:40Z',
            kind: 'assistant_delta',
            content_delta: 'Once you give me a flow and goal, I can create a pending flow run request.',
            segment_id: 'segment-assistant-1',
            segment: {
              id: 'segment-assistant-1',
              turn_id: 'turn-assistant-1',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:28:40Z',
              updated_at: '2026-03-08T19:28:40Z',
              completed_at: null,
              content: 'Once you give me a flow and goal, I can create a pending flow run request.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          created_at: '2026-03-08T19:28:30Z',
          updated_at: '2026-03-08T19:28:43Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Can you use the flow run request too?',
              timestamp: '2026-03-08T19:28:30Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'Once you give me a flow and goal, I can create a pending flow run request.',
              timestamp: '2026-03-08T19:28:30Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          segments: [
            {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T19:28:38Z',
              updated_at: '2026-03-08T19:28:38Z',
              completed_at: null,
              content: 'Considering the flow run request.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
            {
              id: 'segment-assistant-1',
              turn_id: 'turn-assistant-1',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-08T19:28:40Z',
              updated_at: '2026-03-08T19:28:43Z',
              completed_at: '2026-03-08T19:28:43Z',
              content: 'Once you give me a flow and goal, I can create a pending flow run request.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      const text = history.textContent ?? ''
      expect(history).toHaveTextContent(/Considering the flow run request\./)
      expect(history).toHaveTextContent(/pending flow run request\./)
      const reasoningIndex = text.indexOf('Considering the flow run request.')
      const responseIndex = text.indexOf('pending flow run request.')
      expect(reasoningIndex).toBeGreaterThan(-1)
      expect(responseIndex).toBeGreaterThan(-1)
      expect(reasoningIndex).toBeLessThan(responseIndex)
      expect(text.lastIndexOf('pending flow run request.')).toBe(responseIndex)
    })
  })

  it('does not duplicate reasoning summaries when the final snapshot persists them', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return await new Promise<Response>((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Can you use the flow run request too?')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          updated_at: '2026-03-08T21:18:16Z',
          turn: {
            id: 'turn-assistant-1',
            role: 'assistant',
            content: '',
            timestamp: '2026-03-08T21:18:16Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-1',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          updated_at: '2026-03-08T21:18:16Z',
          event: {
            id: 'event-reasoning-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T21:18:16Z',
            kind: 'reasoning_summary',
            content_delta: 'Checking the repo before drafting.',
            segment_id: 'segment-reasoning-1',
            segment: {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T21:18:16Z',
              updated_at: '2026-03-08T21:18:16Z',
              completed_at: null,
              content: 'Checking the repo before drafting.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          },
        })),
      } as MessageEvent)
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the flow run request too?',
          created_at: '2026-03-08T21:18:13Z',
          updated_at: '2026-03-08T21:18:33Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Can you use the flow run request too?',
              timestamp: '2026-03-08T21:18:16Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'I can create a pending flow run request, but there is not yet a concrete project change to draft.',
              timestamp: '2026-03-08T21:18:16Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          segments: [
            {
              id: 'segment-reasoning-1',
              turn_id: 'turn-assistant-1',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-08T21:18:16Z',
              updated_at: '2026-03-08T21:18:16Z',
              completed_at: null,
              content: 'Checking the repo before drafting.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
            {
              id: 'segment-assistant-1',
              turn_id: 'turn-assistant-1',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-08T21:18:33Z',
              updated_at: '2026-03-08T21:18:33Z',
              completed_at: '2026-03-08T21:18:33Z',
              content: 'I can create a pending flow run request, but there is not yet a concrete project change to draft.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      const text = history.textContent ?? ''
      expect(text).toContain('Checking the repo before drafting.')
      expect(text).toContain('I can create a pending flow run request, but there is not yet a concrete project change to draft.')
      expect(text.match(/Checking the repo before drafting\./g)?.length ?? 0).toBe(1)
    })
  })

  it('coalesces interleaved reasoning and assistant deltas until a tool boundary', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(url: string) {
        this.url = url
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return new Promise<Response>(() => {})
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Test interleaved streaming.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectSessionsByPath['/tmp/chat-project']?.conversationId
    expect(conversationId).toBeTruthy()

    await waitFor(() => {
      expect(MockEventSource.instances.length).toBeGreaterThan(0)
    })

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Test interleaved streaming.',
          updated_at: '2026-03-08T20:10:00Z',
          turn: {
            id: 'turn-assistant-interleaved',
            role: 'assistant',
            content: '',
            timestamp: '2026-03-08T20:10:00Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-interleaved',
          },
        }),
      } as MessageEvent)
      const events = [
        {
          id: 'reason-1',
          segment_id: 'segment-reasoning-1',
          sequence: 1,
          kind: 'reasoning_summary',
          timestamp: '2026-03-08T20:10:00Z',
          content_delta: 'Planning from the project context. ',
          segment: {
            id: 'segment-reasoning-1',
            turn_id: 'turn-assistant-interleaved',
            order: 1,
            kind: 'reasoning',
            role: 'assistant',
            status: 'streaming',
            timestamp: '2026-03-08T20:10:00Z',
            updated_at: '2026-03-08T20:10:00Z',
            completed_at: null,
            content: 'Planning from the project context. ',
            artifact_id: null,
            error: null,
            tool_call: null,
            source: null,
          },
        },
        {
          id: 'assistant-1',
          segment_id: 'segment-assistant-1',
          sequence: 2,
          kind: 'assistant_delta',
          timestamp: '2026-03-08T20:10:01Z',
          content_delta: 'I can use the flow run request tool ',
          segment: {
            id: 'segment-assistant-1',
            turn_id: 'turn-assistant-interleaved',
            order: 2,
            kind: 'assistant_message',
            role: 'assistant',
            status: 'streaming',
            timestamp: '2026-03-08T20:10:01Z',
            updated_at: '2026-03-08T20:10:01Z',
            completed_at: null,
            content: 'I can use the flow run request tool ',
            artifact_id: null,
            error: null,
            tool_call: null,
            source: null,
          },
        },
        {
          id: 'reason-2',
          segment_id: 'segment-reasoning-1',
          sequence: 3,
          kind: 'reasoning_summary',
          timestamp: '2026-03-08T20:10:01Z',
          content_delta: 'and I am checking the repository first.',
          segment: {
            id: 'segment-reasoning-1',
            turn_id: 'turn-assistant-interleaved',
            order: 1,
            kind: 'reasoning',
            role: 'assistant',
            status: 'streaming',
            timestamp: '2026-03-08T20:10:00Z',
            updated_at: '2026-03-08T20:10:01Z',
            completed_at: null,
            content: 'Planning from the project context. and I am checking the repository first.',
            artifact_id: null,
            error: null,
            tool_call: null,
            source: null,
          },
        },
        {
          id: 'assistant-2',
          segment_id: 'segment-assistant-1',
          sequence: 4,
          kind: 'assistant_delta',
          timestamp: '2026-03-08T20:10:02Z',
          content_delta: 'once we have a concrete change.',
          segment: {
            id: 'segment-assistant-1',
            turn_id: 'turn-assistant-interleaved',
            order: 2,
            kind: 'assistant_message',
            role: 'assistant',
            status: 'streaming',
            timestamp: '2026-03-08T20:10:01Z',
            updated_at: '2026-03-08T20:10:02Z',
            completed_at: null,
            content: 'I can use the flow run request tool once we have a concrete change.',
            artifact_id: null,
            error: null,
            tool_call: null,
            source: null,
          },
        },
      ]
      for (const event of events) {
        MockEventSource.instances[0]?.onmessage?.({
          data: JSON.stringify(asSegmentUpsertEvent({
            conversation_id: conversationId,
            project_path: '/tmp/chat-project',
            title: 'Test interleaved streaming.',
            updated_at: event.timestamp,
            event: {
              id: event.id,
              turn_id: 'turn-assistant-interleaved',
              sequence: event.sequence,
              timestamp: event.timestamp,
              kind: event.kind,
              content_delta: event.content_delta,
              segment_id: event.segment_id,
              segment: event.segment,
            },
          })),
        } as MessageEvent)
      }
    })

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      expect(history).toHaveTextContent('Planning from the project context. and I am checking the repository first.')
      expect(history).toHaveTextContent('I can use the flow run request tool once we have a concrete change.')
    })
    expect(within(history).getAllByText('Planning from the project context. and I am checking the repository first.')).toHaveLength(1)
  })

  it('reconstructs multiple persisted reasoning segments from snapshot state', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([
            {
              conversation_id: 'conversation-segments',
              project_path: '/tmp/chat-project',
              title: 'Segment thread',
              created_at: '2026-03-13T10:00:00Z',
              updated_at: '2026-03-13T10:00:05Z',
              last_message_preview: 'Here is the grounded plan.',
            },
          ]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/conversation-segments') && !url.includes('/turns')) {
          return new Response(JSON.stringify(withSnapshotSchema({
            conversation_id: 'conversation-segments',
            project_path: '/tmp/chat-project',
            title: 'Segment thread',
            created_at: '2026-03-13T10:00:00Z',
            updated_at: '2026-03-13T10:00:05Z',
            turns: [
              {
                id: 'turn-user-segments',
                role: 'user',
                content: 'Explain your plan.',
                timestamp: '2026-03-13T10:00:00Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
              {
                id: 'turn-assistant-segments',
                role: 'assistant',
                content: 'Here is the grounded plan.',
                timestamp: '2026-03-13T10:00:01Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
            ],
            segments: [
              {
                id: 'segment-reasoning-1',
                turn_id: 'turn-assistant-segments',
                order: 1,
                kind: 'reasoning',
                role: 'assistant',
                status: 'complete',
                timestamp: '2026-03-13T10:00:01Z',
                updated_at: '2026-03-13T10:00:02Z',
                completed_at: '2026-03-13T10:00:02Z',
                content: '**Reviewing repo** Looking through the project layout.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: {
                  app_turn_id: 'app-turn-1',
                  item_id: 'item-rs-1',
                  summary_index: 0,
                  call_id: null,
                },
              },
              {
                id: 'segment-reasoning-2',
                turn_id: 'turn-assistant-segments',
                order: 2,
                kind: 'reasoning',
                role: 'assistant',
                status: 'complete',
                timestamp: '2026-03-13T10:00:03Z',
                updated_at: '2026-03-13T10:00:03Z',
                completed_at: '2026-03-13T10:00:03Z',
                content: '**Considering proposal** Mapping the change to a minimal spec edit.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: {
                  app_turn_id: 'app-turn-1',
                  item_id: 'item-rs-1',
                  summary_index: 1,
                  call_id: null,
                },
              },
              {
                id: 'segment-assistant-1',
                turn_id: 'turn-assistant-segments',
                order: 3,
                kind: 'assistant_message',
                role: 'assistant',
                status: 'complete',
                timestamp: '2026-03-13T10:00:05Z',
                updated_at: '2026-03-13T10:00:05Z',
                completed_at: '2026-03-13T10:00:05Z',
                content: 'Here is the grounded plan.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: {
                  app_turn_id: 'app-turn-1',
                  item_id: 'item-msg-1',
                  summary_index: null,
                  call_id: null,
                },
              },
            ],
            turn_events: [],
            event_log: [],



          })), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response('Not found', { status: 404 })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      expect(history).toHaveTextContent('Reviewing repo')
      expect(history).toHaveTextContent('Considering proposal')
      expect(history).toHaveTextContent('Here is the grounded plan.')
    })
    expect(screen.getAllByTestId(/project-thinking-toggle-/)).toHaveLength(2)
  })

  it('upserts streamed segment payloads onto the matching reasoning card', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(public url: string) {
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([
            {
              conversation_id: 'conversation-segment-upsert',
              project_path: '/tmp/chat-project',
              title: 'Segment upsert thread',
              created_at: '2026-03-13T11:00:00Z',
              updated_at: '2026-03-13T11:00:01Z',
              last_message_preview: 'Planning',
            },
          ]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/conversation-segment-upsert') && !url.includes('/turns')) {
          return new Response(JSON.stringify(withSnapshotSchema({
            conversation_id: 'conversation-segment-upsert',
            project_path: '/tmp/chat-project',
            title: 'Segment upsert thread',
            created_at: '2026-03-13T11:00:00Z',
            updated_at: '2026-03-13T11:00:01Z',
            turns: [
              {
                id: 'turn-user-upsert',
                role: 'user',
                content: 'Draft the change.',
                timestamp: '2026-03-13T11:00:00Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
              {
                id: 'turn-assistant-upsert',
                role: 'assistant',
                content: '',
                timestamp: '2026-03-13T11:00:01Z',
                status: 'streaming',
                kind: 'message',
                artifact_id: null,
              },
            ],
            segments: [
              {
                id: 'segment-reasoning-live',
                turn_id: 'turn-assistant-upsert',
                order: 1,
                kind: 'reasoning',
                role: 'assistant',
                status: 'streaming',
                timestamp: '2026-03-13T11:00:01Z',
                updated_at: '2026-03-13T11:00:01Z',
                completed_at: null,
                content: '**Considering proposal** Drafting the minimal edit.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: {
                  app_turn_id: 'app-turn-2',
                  item_id: 'item-rs-2',
                  summary_index: 0,
                  call_id: null,
                },
              },
            ],
            turn_events: [],
            event_log: [],



          })), {
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

    act(() => {
      useStore.getState().registerProject('/tmp/chat-project')
      useStore.getState().setActiveProjectPath('/tmp/chat-project')
      useStore.getState().setConversationId('conversation-segment-upsert')
    })

    renderProjectsPanel()

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      expect(history).toHaveTextContent('Considering proposal')
    })
    expect(screen.getAllByTestId(/project-thinking-toggle-/)).toHaveLength(1)

    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify(asSegmentUpsertEvent({
          conversation_id: 'conversation-segment-upsert',
          project_path: '/tmp/chat-project',
          title: 'Segment upsert thread',
          updated_at: '2026-03-13T11:00:02Z',
          event: {
            id: 'event-reasoning-live-2',
            turn_id: 'turn-assistant-upsert',
            sequence: 2,
            timestamp: '2026-03-13T11:00:02Z',
            kind: 'reasoning_summary',
            content_delta: ' Mapping the change to a minimal spec edit.',
            segment_id: 'segment-reasoning-live',
            segment: {
              id: 'segment-reasoning-live',
              turn_id: 'turn-assistant-upsert',
              order: 1,
              kind: 'reasoning',
              role: 'assistant',
              status: 'streaming',
              timestamp: '2026-03-13T11:00:01Z',
              updated_at: '2026-03-13T11:00:02Z',
              completed_at: null,
              content: '**Considering proposal** Drafting the minimal edit. Mapping the change to a minimal spec edit.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: {
                app_turn_id: 'app-turn-2',
                item_id: 'item-rs-2',
                summary_index: 0,
                call_id: null,
              },
            },
          },
        })),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(history).toHaveTextContent('Considering proposal')
    })
    expect(screen.getAllByTestId(/project-thinking-toggle-/)).toHaveLength(1)
  })

  it('keeps the composer cleared when sending a turn fails', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && init?.method === 'POST') {
          return new Response(JSON.stringify({ detail: 'backend unavailable' }), {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    useStore.getState().registerProject('/tmp/chat-project')
    useStore.getState().setActiveProjectPath('/tmp/chat-project')

    renderProjectsPanel()

    const composer = screen.getByTestId('project-ai-conversation-input') as HTMLTextAreaElement
    await user.type(composer, 'This should not jump back into the composer.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await waitFor(() => {
      expect(screen.getByText('backend unavailable')).toBeVisible()
    })
    expect(composer).toHaveValue('')
  })

  it('auto-follows at the live edge and shows a jump control when scrolled away', async () => {
    class MockEventSource {
      static instances: MockEventSource[] = []

      onmessage: ((event: MessageEvent) => void) | null = null

      constructor(public url: string) {
        MockEventSource.instances.push(this)
      }

      close() {}
    }

    const user = userEvent.setup()
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify(withSnapshotSchema({
            conversation_id: 'conversation-scroll-1',
            project_path: '/tmp/chat-scroll-project',
            title: 'Scroll thread',
            created_at: '2026-03-06T18:00:00Z',
            updated_at: '2026-03-06T18:00:05Z',
            turns: [
              {
                id: 'turn-user-1',
                role: 'user',
                content: 'First',
                timestamp: '2026-03-06T18:00:00Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
              {
                id: 'turn-assistant-1',
                role: 'assistant',
                content: 'Second',
                timestamp: '2026-03-06T18:00:05Z',
                status: 'complete',
                kind: 'message',
                artifact_id: null,
              },
            ],
            turn_events: [],
            event_log: [],



          })), {
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

    act(() => {
      useStore.getState().registerProject('/tmp/chat-scroll-project')
      useStore.getState().setActiveProjectPath('/tmp/chat-scroll-project')
      useStore.getState().updateProjectSessionState('/tmp/chat-scroll-project', {
        conversationId: 'conversation-scroll-1',
      })
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Second')
    })

    const conversationBody = screen.getByTestId('project-ai-conversation-body') as HTMLDivElement
    let scrollTop = 200
    let scrollHeight = 400
    Object.defineProperty(conversationBody, 'clientHeight', {
      configurable: true,
      get: () => 200,
    })
    Object.defineProperty(conversationBody, 'scrollHeight', {
      configurable: true,
      get: () => scrollHeight,
    })
    Object.defineProperty(conversationBody, 'scrollTop', {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = value
      },
    })
    conversationBody.scrollTo = vi.fn(({ top }: ScrollToOptions) => {
      if (typeof top === 'number') {
        scrollTop = top
      }
    })

    fireEvent.scroll(conversationBody)

    scrollHeight = 520
    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: 'conversation-scroll-1',
          project_path: '/tmp/chat-scroll-project',
          title: 'Scroll thread',
          updated_at: '2026-03-06T18:00:10Z',
          turn: {
            id: 'turn-assistant-2',
            role: 'assistant',
            content: 'Third',
            timestamp: '2026-03-06T18:00:10Z',
            status: 'complete',
            kind: 'message',
            artifact_id: null,
          },
        }),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(scrollTop).toBe(520)
    })
    expect(screen.queryByTestId('project-ai-conversation-jump-to-bottom')).not.toBeInTheDocument()

    scrollTop = 120
    fireEvent.scroll(conversationBody)

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-jump-to-bottom')).toBeVisible()
    })

    scrollHeight = 640
    act(() => {
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_upsert',
          conversation_id: 'conversation-scroll-1',
          project_path: '/tmp/chat-scroll-project',
          title: 'Scroll thread',
          updated_at: '2026-03-06T18:00:15Z',
          turn: {
            id: 'turn-assistant-3',
            role: 'assistant',
            content: 'Fourth',
            timestamp: '2026-03-06T18:00:15Z',
            status: 'complete',
            kind: 'message',
            artifact_id: null,
          },
        }),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-jump-to-bottom')).toBeVisible()
    })
    expect(scrollTop).toBe(120)

    await user.click(screen.getByTestId('project-ai-conversation-jump-to-bottom'))

    expect(conversationBody.scrollTo).toHaveBeenCalled()
    expect(scrollTop).toBe(640)
  })

  it('switches between project threads without mixing their histories', async () => {
    const user = userEvent.setup()
    const conversationSummaries = [
      {
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T15:00:00Z',
        updated_at: '2026-03-07T15:10:00Z',
        last_message_preview: 'This is the planning thread history.',
      },
      {
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T14:00:00Z',
        updated_at: '2026-03-07T14:05:00Z',
        last_message_preview: 'Discuss the design changes.',
      },
    ]
    const conversationSnapshots = {
      'conversation-thread-a': withSnapshotSchema({
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T14:00:00Z',
        updated_at: '2026-03-07T14:05:00Z',
        turns: [
          {
            id: 'turn-a-1',
            role: 'user',
            content: 'Discuss the design changes.',
            timestamp: '2026-03-07T14:05:00Z',
            kind: 'message',
            artifact_id: null,
          },
        ],
        event_log: [],



      }),
      'conversation-thread-b': withSnapshotSchema({
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T15:00:00Z',
        updated_at: '2026-03-07T15:10:00Z',
        turns: [
          {
            id: 'turn-b-1',
            role: 'assistant',
            content: 'This is the planning thread history.',
            timestamp: '2026-03-07T15:10:00Z',
            kind: 'message',
            artifact_id: null,
          },
        ],
        event_log: [],



      }),
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(
            JSON.stringify(conversationSummaries),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        const conversationIdMatch = url.match(/\/api\/conversations\/([^/?]+)/)
        const conversationId = conversationIdMatch ? decodeURIComponent(conversationIdMatch[1]!) : null
        if (conversationId && !url.includes('/turns')) {
          return new Response(JSON.stringify(conversationSnapshots[conversationId as keyof typeof conversationSnapshots]), {
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

    act(() => {
      useStore.getState().registerProject('/tmp/thread-project')
      useStore.getState().setActiveProjectPath('/tmp/thread-project')
      useStore.getState().setConversationId('conversation-thread-a')
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-thread-list')).toHaveTextContent('Design thread')
    })
    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Discuss the design changes.')
    })

    await user.click(await screen.findByRole('button', { name: /Open thread Planning thread/i }))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Discuss the design changes.')
  })

  it('does not switch back to a thread when an in-flight send resolves after the user selects another thread', async () => {
    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null
    const conversationSummaries = [
      {
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T15:00:00Z',
        updated_at: '2026-03-07T15:10:00Z',
        last_message_preview: 'This is the planning thread history.',
      },
      {
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T14:00:00Z',
        updated_at: '2026-03-07T14:05:00Z',
        last_message_preview: 'Discuss the design changes.',
      },
    ]

    const conversationSnapshots = {
      'conversation-thread-a': withSnapshotSchema({
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T14:00:00Z',
        updated_at: '2026-03-07T14:05:00Z',
        turns: [
          {
            id: 'turn-a-1',
            role: 'user',
            content: 'Discuss the design changes.',
            timestamp: '2026-03-07T14:05:00Z',
            kind: 'message',
            artifact_id: null,
          },
        ],
        event_log: [],



      }),
      'conversation-thread-b': withSnapshotSchema({
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T15:00:00Z',
        updated_at: '2026-03-07T15:10:00Z',
        turns: [
          {
            id: 'turn-b-1',
            role: 'assistant',
            content: 'This is the planning thread history.',
            timestamp: '2026-03-07T15:10:00Z',
            kind: 'message',
            artifact_id: null,
          },
        ],
        event_log: [],



      }),
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(
            JSON.stringify(conversationSummaries),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        const conversationIdMatch = url.match(/\/api\/conversations\/([^/?]+)/)
        const conversationId = conversationIdMatch ? decodeURIComponent(conversationIdMatch[1]!) : null
        if (conversationId && !url.includes('/turns')) {
          return new Response(JSON.stringify(conversationSnapshots[conversationId as keyof typeof conversationSnapshots]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (conversationId === 'conversation-thread-a' && init?.method === 'POST') {
          return new Promise((resolve) => {
            resolveTurnResponse = resolve
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )

    act(() => {
      useStore.getState().registerProject('/tmp/thread-project')
      useStore.getState().setActiveProjectPath('/tmp/thread-project')
      useStore.getState().setConversationId('conversation-thread-a')
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Discuss the design changes.')
    })

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Follow up on thread A.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await user.click(await screen.findByRole('button', { name: /Open thread Planning thread/i }))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify(withSnapshotSchema({
          conversation_id: 'conversation-thread-a',
          project_path: '/tmp/thread-project',
          title: 'Design thread',
          created_at: '2026-03-07T14:00:00Z',
          updated_at: '2026-03-07T15:20:00Z',
          turns: [
            {
              id: 'turn-a-1',
              role: 'user',
              content: 'Discuss the design changes.',
              timestamp: '2026-03-07T14:05:00Z',
              kind: 'message',
              artifact_id: null,
              status: 'complete',
            },
            {
              id: 'turn-a-2',
              role: 'user',
              content: 'Follow up on thread A.',
              timestamp: '2026-03-07T15:19:58Z',
              kind: 'message',
              artifact_id: null,
              status: 'complete',
            },
            {
              id: 'turn-a-3',
              role: 'assistant',
              content: 'Response for thread A.',
              timestamp: '2026-03-07T15:20:00Z',
              kind: 'message',
              artifact_id: null,
              status: 'complete',
            },
          ],
          turn_events: [
            {
              id: 'event-a-complete',
              turn_id: 'turn-a-3',
              sequence: 1,
              timestamp: '2026-03-07T15:20:00Z',
              kind: 'assistant_completed',
              message: 'Assistant turn completed.',
            },
          ],
          event_log: [],



        })),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Open thread Planning thread/i })).toHaveAttribute('aria-current', 'true')
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Response for thread A.')
  })

  it('deletes the active thread and falls back to the next remaining thread', async () => {
    const user = userEvent.setup()
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true)
    let conversationSummaries = [
      {
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T12:10:00Z',
        updated_at: '2026-03-07T12:40:00Z',
        last_message_preview: 'This is the planning thread history.',
      },
      {
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T12:00:00Z',
        updated_at: '2026-03-07T12:30:00Z',
        last_message_preview: 'Discuss the design changes.',
      },
    ]

    const conversationSnapshots = {
      'conversation-thread-a': withSnapshotSchema({
        conversation_id: 'conversation-thread-a',
        project_path: '/tmp/thread-project',
        title: 'Design thread',
        created_at: '2026-03-07T12:00:00Z',
        updated_at: '2026-03-07T12:30:00Z',
        turns: [
          {
            id: 'turn-a-1',
            role: 'user',
            kind: 'message',
            status: 'complete',
            content: 'Discuss the design changes.',
            timestamp: '2026-03-07T12:30:00Z',
          },
        ],
        turn_events: [],
        event_log: [],
      }),
      'conversation-thread-b': withSnapshotSchema({
        conversation_id: 'conversation-thread-b',
        project_path: '/tmp/thread-project',
        title: 'Planning thread',
        created_at: '2026-03-07T12:10:00Z',
        updated_at: '2026-03-07T12:40:00Z',
        turns: [
          {
            id: 'turn-b-1',
            role: 'assistant',
            kind: 'message',
            status: 'complete',
            content: 'This is the planning thread history.',
            timestamp: '2026-03-07T12:40:00Z',
          },
        ],
        turn_events: [],
        event_log: [],
      }),
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return new Response(JSON.stringify(conversationSummaries), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        const conversationIdMatch = url.match(/\/api\/conversations\/([^/?]+)/)
        const conversationId = conversationIdMatch ? decodeURIComponent(conversationIdMatch[1]!) : null
        if (conversationId && !url.includes('/turns') && init?.method === 'DELETE') {
          conversationSummaries = conversationSummaries.filter((entry) => entry.conversation_id !== conversationId)
          return new Response(JSON.stringify({
            status: 'deleted',
            conversation_id: conversationId,
            project_path: '/tmp/thread-project',
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (conversationId && !url.includes('/turns')) {
          return new Response(JSON.stringify(conversationSnapshots[conversationId as keyof typeof conversationSnapshots]), {
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

    act(() => {
      useStore.getState().registerProject('/tmp/thread-project')
      useStore.getState().setActiveProjectPath('/tmp/thread-project')
      useStore.getState().setConversationId('conversation-thread-a')
    })

    renderProjectsPanel()

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Discuss the design changes.')
    })

    await user.click(screen.getByTestId('project-thread-delete-conversation-thread-a'))

    expect(confirmSpy).toHaveBeenCalledWith('Delete thread "Design thread"?')

    await waitFor(() => {
      expect(screen.getByTestId('project-thread-list')).not.toHaveTextContent('Design thread')
    })
    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    })
  })

})
