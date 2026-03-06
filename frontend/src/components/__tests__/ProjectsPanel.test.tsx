import { ProjectsPanel } from '@/components/ProjectsPanel'
import { useStore } from '@/store'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
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
      vi.fn(async () =>
        new Response(JSON.stringify({ branch: 'main' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      ),
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

  it('shows an error when picker selection cannot resolve an absolute project path', async () => {
    render(<ProjectsPanel />)
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

  it('registers a selected directory from the project new-button picker', async () => {
    const user = userEvent.setup()
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
    expect(screen.getByTestId('projects-list')).toHaveTextContent('/tmp/quick-switch-project')
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
              kind: 'message',
              artifact_id: null,
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              content: 'Visible.',
              timestamp: '2026-03-06T21:45:02Z',
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
      expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Visible.')
    })
  })
})
