import { ProjectsPanel } from '@/components/ProjectsPanel'
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

const resetProjectScopeState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {},
    projectScopedWorkspaces: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
  }))
}

describe('ProjectsPanel', () => {
  beforeEach(() => {
    setViewportWidth(DEFAULT_VIEWPORT_WIDTH)
    resetProjectScopeState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ status: 'canceled' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('renders project controls and event log', () => {
    render(<ProjectsPanel />)

    expect(screen.getByText('Projects')).toBeVisible()
    expect(screen.getByTestId('quick-switch-new-button')).toBeVisible()
    expect(screen.getByTestId('project-directory-picker-input')).toBeInTheDocument()
    expect(screen.getByTestId('quick-switch-controls')).toBeVisible()
    expect(screen.getByTestId('projects-list')).toBeVisible()
    expect(screen.getByTestId('project-event-log-surface')).toBeVisible()
  })

  it('lets the operator resize sidebar sections in desktop layout', () => {
    render(<ProjectsPanel />)

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

  it('shows an error when the browser fallback cannot resolve an absolute project path', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ detail: 'picker unavailable' }), {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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
    render(<ProjectsPanel />)
    await userEvent.setup().click(screen.getByTestId('quick-switch-new-button'))
    const pickerInput = screen.getByTestId('project-directory-picker-input') as HTMLInputElement
    const selectedFile = new File(['console.log("hello")'], 'main.ts', { type: 'text/plain' })
    Object.defineProperty(selectedFile, 'webkitRelativePath', {
      configurable: true,
      value: 'quick-switch-project/src/main.ts',
    })
    fireEvent.change(pickerInput, {
      target: {
        files: [selectedFile],
      },
    })
    expect(screen.getByTestId('project-registration-error')).toHaveTextContent(
      'Unable to resolve an absolute project path from the selected directory.',
    )
  })

  it('registers a project from the native directory picker', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ status: 'selected', directory_path: '/tmp/quick-switch-project' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.endsWith('/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/register')) {
          return new Response(JSON.stringify({
            project_id: 'quick-switch-project-1234',
            project_path: '/tmp/quick-switch-project',
            display_name: 'quick-switch-project',
            is_favorite: false,
            active_conversation_id: null,
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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
    const user = userEvent.setup()
    render(<ProjectsPanel />)
    await user.click(screen.getByTestId('quick-switch-new-button'))

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/quick-switch-project']).toBeDefined()
    })
    expect(useStore.getState().activeProjectPath).toBe('/tmp/quick-switch-project')
  })

  it('falls back to the browser directory picker when the native picker is unavailable', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ detail: 'picker unavailable' }), {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.endsWith('/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/register')) {
          return new Response(JSON.stringify({
            project_id: 'quick-switch-project-1234',
            project_path: '/tmp/quick-switch-project',
            display_name: 'quick-switch-project',
            is_favorite: false,
            active_conversation_id: null,
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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
    render(<ProjectsPanel />)

    const pickerInput = screen.getByTestId('project-directory-picker-input') as HTMLInputElement
    const pickerClickSpy = vi.spyOn(pickerInput, 'click')
    await user.click(screen.getByTestId('quick-switch-new-button'))
    expect(pickerClickSpy).toHaveBeenCalled()

    const selectedFile = new File(['console.log("hello")'], 'main.ts', { type: 'text/plain' })
    Object.defineProperty(selectedFile, 'path', {
      configurable: true,
      value: '/tmp/quick-switch-project/src/main.ts',
    })
    Object.defineProperty(selectedFile, 'webkitRelativePath', {
      configurable: true,
      value: 'quick-switch-project/src/main.ts',
    })

    fireEvent.change(pickerInput, {
      target: {
        files: [selectedFile],
      },
    })

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/quick-switch-project']).toBeDefined()
    })
    expect(screen.getByTestId('projects-list')).toHaveTextContent('quick-switch-project')
    expect(useStore.getState().activeProjectPath).toBe('/tmp/quick-switch-project')
  })

  it('renders the user turn before the assistant response completes', async () => {
    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Show this message immediately.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Show this message immediately.')
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Thinking...')
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Sending...')

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
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
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Visible.')
    })
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Send')
  })

  it('renders the assistant reply even when the backend canonicalizes project_path', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
          return new Response(
            JSON.stringify({
              conversation_id: 'conversation-home-project-1',
              project_path: '/System/Volumes/Data/home/chris/tinker/sparkspawn',
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
              spec_edit_proposals: [],
              execution_cards: [],
              execution_workflow: {
                run_id: null,
                status: 'idle',
                error: null,
                flow_source: null,
              },
            }),
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

    useStore.getState().registerProject('/home/chris/tinker/sparkspawn')
    useStore.getState().setActiveProjectPath('/home/chris/tinker/sparkspawn')

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Reply with a one-line acknowledgement only.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Acknowledged.')
    })
  })

  it('renders assistant tool calls before the completed assistant summary for the same turn', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(
            JSON.stringify({
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
              turn_events: [
                {
                  id: 'event-tool-started-1',
                  turn_id: 'turn-assistant-1',
                  sequence: 1,
                  timestamp: '2026-03-07T19:46:10Z',
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
                },
                {
                  id: 'event-tool-completed-1',
                  turn_id: 'turn-assistant-1',
                  sequence: 2,
                  timestamp: '2026-03-07T19:46:12Z',
                  kind: 'tool_call_completed',
                  tool_call_id: 'tool-ls',
                  tool_call: {
                    id: 'tool-ls',
                    kind: 'command_execution',
                    status: 'completed',
                    title: 'Run command',
                    command: '/bin/zsh -lc ls',
                    output: 'AGENTS.md',
                    file_paths: [],
                  },
                },
                {
                  id: 'event-tool-started-2',
                  turn_id: 'turn-assistant-1',
                  sequence: 3,
                  timestamp: '2026-03-07T19:46:20Z',
                  kind: 'tool_call_started',
                  tool_call_id: 'tool-ps',
                  tool_call: {
                    id: 'tool-ps',
                    kind: 'command_execution',
                    status: 'running',
                    title: 'Run command',
                    command: '/bin/zsh -lc ps',
                    output: null,
                    file_paths: [],
                  },
                },
                {
                  id: 'event-tool-completed-2',
                  turn_id: 'turn-assistant-1',
                  sequence: 4,
                  timestamp: '2026-03-07T19:46:22Z',
                  kind: 'tool_call_completed',
                  tool_call_id: 'tool-ps',
                  tool_call: {
                    id: 'tool-ps',
                    kind: 'command_execution',
                    status: 'completed',
                    title: 'Run command',
                    command: '/bin/zsh -lc ps',
                    output: 'PID TTY TIME CMD',
                    file_paths: [],
                  },
                },
                {
                  id: 'event-assistant-completed-1',
                  turn_id: 'turn-assistant-1',
                  sequence: 5,
                  timestamp: '2026-03-07T19:46:30Z',
                  kind: 'assistant_completed',
                  message: 'Assistant turn completed.',
                },
              ],
              event_log: [],
              spec_edit_proposals: [],
              execution_cards: [],
              execution_workflow: {
                run_id: null,
                status: 'idle',
                error: null,
                flow_source: null,
              },
            }),
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

    render(<ProjectsPanel />)

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    const text = history.textContent ?? ''

    expect(text.indexOf('/bin/zsh -lc ls')).toBeGreaterThan(-1)
    expect(text.indexOf('/bin/zsh -lc ps')).toBeGreaterThan(-1)
    expect(text.indexOf('Summary after tools.')).toBeGreaterThan(-1)
    expect(text.indexOf('/bin/zsh -lc ls')).toBeLessThan(text.indexOf('Summary after tools.'))
    expect(text.indexOf('/bin/zsh -lc ps')).toBeLessThan(text.indexOf('Summary after tools.'))
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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Stream this reply.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectScopedWorkspaces['/tmp/chat-project']?.conversationId
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
        data: JSON.stringify({
          type: 'turn_event',
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
        }),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Working on')
    })
    expect(MockEventSource.instances).toHaveLength(1)
    expect(screen.getByTestId('project-ai-conversation-send-button')).toHaveTextContent('Thinking...')

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
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
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Draft a spec.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectScopedWorkspaces['/tmp/chat-project']?.conversationId
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
        data: JSON.stringify({
          type: 'turn_event',
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
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_event',
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
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_event',
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
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_event',
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
          },
        }),
      } as MessageEvent)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Scanning the repository structure first.')
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('I found the main entry points and can summarize them.')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
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
          turn_events: [
            {
              id: 'event-reasoning-1',
              turn_id: 'turn-assistant-1',
              sequence: 1,
              timestamp: '2026-03-08T19:10:00Z',
              kind: 'reasoning_summary',
              content_delta: 'Scanning the repository structure first.',
            },
            {
              id: 'event-tool-started-1',
              turn_id: 'turn-assistant-1',
              sequence: 2,
              timestamp: '2026-03-08T19:10:02Z',
              kind: 'tool_call_completed',
              tool_call_id: 'tool-ls',
              tool_call: {
                id: 'tool-ls',
                kind: 'command_execution',
                status: 'completed',
                title: 'Run command',
                command: '/bin/zsh -lc ls',
                output: 'README.md',
                file_paths: [],
              },
            },
            {
              id: 'event-assistant-completed-1',
              turn_id: 'turn-assistant-1',
              sequence: 3,
              timestamp: '2026-03-08T19:10:04Z',
              kind: 'assistant_completed',
              message: 'Assistant turn completed.',
            },
          ],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Can you use the spec proposal too?')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectScopedWorkspaces['/tmp/chat-project']?.conversationId
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
          title: 'Can you use the spec proposal too?',
          updated_at: '2026-03-08T19:28:40Z',
          turn: {
            id: 'turn-assistant-1',
            role: 'assistant',
            content: 'Yes. Once you give me a concrete user story or spec change, I can call `draft_spec_proposal`.',
            timestamp: '2026-03-08T19:28:40Z',
            status: 'streaming',
            kind: 'message',
            artifact_id: null,
            parent_turn_id: 'turn-user-1',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_event',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the spec proposal too?',
          updated_at: '2026-03-08T19:28:38Z',
          event: {
            id: 'event-reasoning-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T19:28:38Z',
            kind: 'reasoning_summary',
            content_delta: 'Considering the spec proposal.',
          },
        }),
      } as MessageEvent)
      MockEventSource.instances[0]?.onmessage?.({
        data: JSON.stringify({
          type: 'turn_event',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the spec proposal too?',
          updated_at: '2026-03-08T19:28:40Z',
          event: {
            id: 'event-assistant-delta-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T19:28:40Z',
            kind: 'assistant_delta',
            content_delta: 'Yes. Once you give me a concrete user story or spec change, I can call `draft_spec_proposal`.',
          },
        }),
      } as MessageEvent)
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the spec proposal too?',
          created_at: '2026-03-08T19:28:30Z',
          updated_at: '2026-03-08T19:28:43Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Can you use the spec proposal too?',
              timestamp: '2026-03-08T19:28:30Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'Yes. Once you give me a concrete user story or spec change, I can call `draft_spec_proposal`.',
              timestamp: '2026-03-08T19:28:30Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          turn_events: [
            {
              id: 'event-reasoning-1',
              turn_id: 'turn-assistant-1',
              sequence: 1,
              timestamp: '2026-03-08T19:28:38Z',
              kind: 'reasoning_summary',
              content_delta: 'Considering the spec proposal.',
            },
            {
              id: 'event-assistant-completed-1',
              turn_id: 'turn-assistant-1',
              sequence: 1,
              timestamp: '2026-03-08T19:28:43Z',
              kind: 'assistant_completed',
              message: 'Assistant turn completed.',
            },
          ],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    )

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      const text = history.textContent ?? ''
      const firstResponse = text.indexOf('Yes. Once you give me a concrete user story or spec change')
      expect(firstResponse).toBeGreaterThan(-1)
      expect(text.indexOf('Considering the spec proposal.')).toBeGreaterThan(-1)
      expect(text.indexOf('Considering the spec proposal.')).toBeLessThan(firstResponse)
      expect(text.lastIndexOf('Yes. Once you give me a concrete user story or spec change')).toBe(firstResponse)
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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Can you use the spec proposal too?')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectScopedWorkspaces['/tmp/chat-project']?.conversationId
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
          title: 'Can you use the spec proposal too?',
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
        data: JSON.stringify({
          type: 'turn_event',
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the spec proposal too?',
          updated_at: '2026-03-08T21:18:16Z',
          event: {
            id: 'event-reasoning-1',
            turn_id: 'turn-assistant-1',
            sequence: 1,
            timestamp: '2026-03-08T21:18:16Z',
            kind: 'reasoning_summary',
            content_delta: 'Checking the repo before drafting.',
          },
        }),
      } as MessageEvent)
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
          conversation_id: conversationId,
          project_path: '/tmp/chat-project',
          title: 'Can you use the spec proposal too?',
          created_at: '2026-03-08T21:18:13Z',
          updated_at: '2026-03-08T21:18:33Z',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              content: 'Can you use the spec proposal too?',
              timestamp: '2026-03-08T21:18:16Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'I can use `draft_spec_proposal`, but there is not yet a concrete project change to draft.',
              timestamp: '2026-03-08T21:18:16Z',
              status: 'complete',
              kind: 'message',
              artifact_id: null,
              parent_turn_id: 'turn-user-1',
            },
          ],
          turn_events: [
            {
              id: 'event-reasoning-1',
              turn_id: 'turn-assistant-1',
              sequence: 1,
              timestamp: '2026-03-08T21:18:16Z',
              kind: 'reasoning_summary',
              content_delta: 'Checking the repo before drafting.',
            },
            {
              id: 'event-assistant-completed-1',
              turn_id: 'turn-assistant-1',
              sequence: 2,
              timestamp: '2026-03-08T21:18:33Z',
              kind: 'assistant_completed',
              message: 'Assistant turn completed.',
            },
          ],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
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
      expect(text).toContain('I can use `draft_spec_proposal`, but there is not yet a concrete project change to draft.')
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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({ detail: 'Unknown conversation' }), {
            status: 404,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
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

    render(<ProjectsPanel />)

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Test interleaved streaming.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    const conversationId = useStore.getState().projectScopedWorkspaces['/tmp/chat-project']?.conversationId
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
          sequence: 1,
          kind: 'reasoning_summary',
          timestamp: '2026-03-08T20:10:00Z',
          content_delta: 'Planning from the project context. ',
        },
        {
          id: 'assistant-1',
          sequence: 2,
          kind: 'assistant_delta',
          timestamp: '2026-03-08T20:10:01Z',
          content_delta: 'I can use the spec proposal tool ',
        },
        {
          id: 'reason-2',
          sequence: 3,
          kind: 'reasoning_summary',
          timestamp: '2026-03-08T20:10:01Z',
          content_delta: 'and I am checking the repository first.',
        },
        {
          id: 'assistant-2',
          sequence: 4,
          kind: 'assistant_delta',
          timestamp: '2026-03-08T20:10:02Z',
          content_delta: 'once we have a concrete change.',
        },
      ]
      for (const event of events) {
        MockEventSource.instances[0]?.onmessage?.({
          data: JSON.stringify({
            type: 'turn_event',
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
            },
          }),
        } as MessageEvent)
      }
    })

    const history = await screen.findByTestId('project-ai-conversation-history-list')
    await waitFor(() => {
      expect(history).toHaveTextContent('Planning from the project context. and I am checking the repository first.')
      expect(history).toHaveTextContent('I can use the spec proposal tool once we have a concrete change.')
    })
    expect(within(history).getAllByText('Thinking')).toHaveLength(1)
  })

  it('keeps the composer cleared when sending a turn fails', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && init?.method === 'POST') {
          return new Response(JSON.stringify({ detail: 'backend unavailable' }), {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
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

    render(<ProjectsPanel />)

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
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/conversations/') && !init?.method) {
          return new Response(JSON.stringify({
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
            spec_edit_proposals: [],
            execution_cards: [],
            execution_workflow: {
              run_id: null,
              status: 'idle',
              error: null,
              flow_source: null,
            },
          }), {
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
      useStore.getState().updateProjectScopedWorkspace('/tmp/chat-scroll-project', {
        conversationId: 'conversation-scroll-1',
      })
    })

    render(<ProjectsPanel />)

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
    let conversationSummaries = [
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
      'conversation-thread-a': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: {
          run_id: null,
          status: 'idle',
          error: null,
          flow_source: null,
        },
      },
      'conversation-thread-b': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: {
          run_id: null,
          status: 'idle',
          error: null,
          flow_source: null,
        },
      },
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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

    render(<ProjectsPanel />)

    await waitFor(() => {
      expect(screen.getByTestId('project-thread-list')).toHaveTextContent('Design thread')
    })
    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Discuss the design changes.')
    })

    await user.click(screen.getByRole('button', { name: /Open thread Planning thread/i }))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    })
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Discuss the design changes.')
  })

  it('does not switch back to a thread when an in-flight send resolves after the user selects another thread', async () => {
    const user = userEvent.setup()
    let resolveTurnResponse: ((response: Response) => void) | null = null
    let conversationSummaries = [
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
      'conversation-thread-a': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: {
          run_id: null,
          status: 'idle',
          error: null,
          flow_source: null,
        },
      },
      'conversation-thread-b': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: {
          run_id: null,
          status: 'idle',
          error: null,
          flow_source: null,
        },
      },
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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

    render(<ProjectsPanel />)

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Discuss the design changes.')
    })

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Follow up on thread A.')
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))

    await user.click(screen.getByRole('button', { name: /Open thread Planning thread/i }))

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('This is the planning thread history.')
    })

    resolveTurnResponse?.(
      new Response(
        JSON.stringify({
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
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: {
            run_id: null,
            status: 'idle',
            error: null,
            flow_source: null,
          },
        }),
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
      'conversation-thread-a': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: { status: 'idle' },
      },
      'conversation-thread-b': {
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
        spec_edit_proposals: [],
        execution_cards: [],
        execution_workflow: { status: 'idle' },
      },
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/projects/conversations')) {
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

    render(<ProjectsPanel />)

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
