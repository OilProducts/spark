import App from '@/App'
import { useStore } from '@/store'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const DEFAULT_EDITOR_SIDEBAR_WIDTH = 288
const LINEAR_FLOW_NAME = 'test-linear.dot'
const REVIEW_FLOW_NAME = 'test-review-loop.dot'
const NESTED_LINEAR_FLOW_NAME = 'team/review/test-linear.dot'

const setViewportWidth = (width: number) => {
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: width,
  })
  window.dispatchEvent(new Event('resize'))
}

const resetAppShellState = () => {
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
    runRecordOverrides: {},
    workingDir: DEFAULT_WORKING_DIRECTORY,
    runtimeStatus: 'idle',
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    logs: [],
    humanGate: null,
    nodeStatuses: {},
    selectedNodeId: null,
    selectedEdgeId: null,
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    graphAttrs: {},
    graphAttrErrors: {},
    editorSidebarWidth: DEFAULT_EDITOR_SIDEBAR_WIDTH,
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
  }))
}

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

const jsonResponse = (payload: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  })

const buildProjectRecord = (projectPath: string) => ({
  project_id: projectPath.split('/').filter(Boolean).join('-'),
  project_path: projectPath,
  display_name: projectPath.split('/').filter(Boolean).at(-1) ?? projectPath,
  created_at: '2026-03-25T12:00:00Z',
  last_opened_at: '2026-03-25T12:00:00Z',
  last_accessed_at: null,
  is_favorite: false,
  active_conversation_id: null,
})

const buildConversationSummary = ({
  conversationId,
  projectPath,
  title,
}: {
  conversationId: string
  projectPath: string
  title: string
}) => ({
  conversation_id: conversationId,
  conversation_handle: null,
  project_path: projectPath,
  title,
  created_at: '2026-03-25T12:00:00Z',
  updated_at: '2026-03-25T12:05:00Z',
  last_message_preview: null,
})

const buildRunRecord = ({
  flowName,
  projectPath,
  runId,
}: {
  flowName: string
  projectPath: string
  runId: string
}) => ({
  run_id: runId,
  flow_name: flowName,
  status: 'completed',
  outcome: 'success',
  outcome_reason_code: null,
  outcome_reason_message: null,
  working_directory: projectPath,
  project_path: projectPath,
  git_branch: 'main',
  git_commit: 'abcdef0',
  spec_id: null,
  plan_id: null,
  model: 'gpt-5.3-codex-spark',
  started_at: '2026-03-25T12:00:00Z',
  ended_at: '2026-03-25T12:05:00Z',
  last_error: null,
  token_usage: 1234,
})

const installCanvasWorkspaceFetchMock = () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      if (url.includes(`/attractor/api/flows/${LINEAR_FLOW_NAME}`)) {
        return new Response(JSON.stringify({
          name: LINEAR_FLOW_NAME,
          content: [
            'digraph simple_linear {',
            '  graph [label="Simple Linear Workflow", goal="Inspect the repo."];',
            '  start [shape=Mdiamond, label="Start"];',
            '  plan [shape=box, label="Plan", prompt="Plan the work."];',
            '  done [shape=Msquare, label="Done"];',
            '  start -> plan;',
            '  plan -> done;',
            '}',
          ].join('\n'),
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes(`/attractor/api/flows/${REVIEW_FLOW_NAME}`)) {
        return new Response(JSON.stringify({
          name: REVIEW_FLOW_NAME,
          content: [
            'digraph implement_review_loop {',
            '  graph [',
            '    "spark.launch_inputs"="[{\\"key\\":\\"context.request.summary\\",\\"label\\":\\"Request Summary\\",\\"type\\":\\"string\\",\\"required\\":true}]",',
            '    label="Implementation Review Loop"',
            '  ];',
            '  start [shape=Mdiamond, label="Start"];',
            '  request [shape=box, label="Request", prompt="Review request."];',
            '  done [shape=Msquare, label="Done"];',
            '  start -> request;',
            '  request -> done;',
            '}',
          ].join('\n'),
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/attractor/api/flows')) {
        return new Response(JSON.stringify([LINEAR_FLOW_NAME, REVIEW_FLOW_NAME]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/attractor/preview') && init?.method === 'POST') {
        const dot = typeof init.body === 'string' ? init.body : ''
        if (dot.includes('implement_review_loop')) {
          return new Response(JSON.stringify({
            status: 'ok',
            graph: {
              graph_attrs: {
                label: 'Implementation Review Loop',
                goal: null,
                'spark.launch_inputs': JSON.stringify([
                  {
                    key: 'context.request.summary',
                    label: 'Request Summary',
                    type: 'string',
                    required: true,
                  },
                ]),
              },
              nodes: [
                { id: 'start', label: 'Start', shape: 'Mdiamond' },
                { id: 'request', label: 'Request', shape: 'box', prompt: 'Review request.' },
                { id: 'done', label: 'Done', shape: 'Msquare' },
              ],
              edges: [
                { from: 'start', to: 'request', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
                { from: 'request', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              ],
            },
            diagnostics: [],
            errors: [],
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response(JSON.stringify({
          status: 'ok',
          graph: {
            graph_attrs: {
              label: 'Simple Linear Workflow',
              goal: 'Inspect the repo.',
              'spark.title': 'Simple Linear Workflow',
            },
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'plan', label: 'Plan', shape: 'box', prompt: 'Plan the work.' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'plan', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              { from: 'plan', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/workspace/api/projects')) {
        return new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/workspace/api/projects/metadata')) {
        return new Response(JSON.stringify({ branch: 'main' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/attractor/status')) {
        return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/attractor/runs')) {
        return new Response(JSON.stringify({ runs: [] }), {
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
}

describe('App shell behavior', () => {
  beforeEach(() => {
    resetAppShellState()
    class MockEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = MockEventSource.OPEN
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
      }

      close() {
        this.readyState = MockEventSource.CLOSED
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/attractor/api/flows')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/runs')) {
          return new Response(JSON.stringify({ runs: [] }), {
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

  it('renders shell regions and switches among projects/triggers/settings/runs modes', async () => {
    const user = userEvent.setup()
    render(<App />)

    expect(screen.getByTestId('app-shell')).toBeVisible()
    expect(screen.getByTestId('app-main')).toBeVisible()
    expect(screen.getByTestId('top-nav')).toBeVisible()
    expect(screen.getByTestId('projects-panel')).toBeVisible()
    expect(screen.getByTestId('top-nav-project-switcher')).toBeVisible()
    expect(screen.getByTestId('top-nav-project-add-button')).toBeVisible()
    expect(screen.getByTestId('top-nav-project-clear-button')).toBeDisabled()
    expect(screen.queryByTestId('top-nav-active-flow')).not.toBeInTheDocument()
    expect(screen.queryByTestId('top-nav-run-context')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('nav-mode-triggers'))
    expect(useStore.getState().viewMode).toBe('triggers')
    expect(screen.getByTestId('triggers-panel')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-settings'))
    expect(useStore.getState().viewMode).toBe('settings')
    expect(screen.getByTestId('settings-panel')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-runs'))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(screen.getByTestId('runs-panel')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-projects'))
    expect(useStore.getState().viewMode).toBe('home')
    expect(screen.getByTestId('projects-panel')).toBeVisible()
  })

  it('allows editor navigation without an active project and keeps the empty state visible', async () => {
    const user = userEvent.setup()
    render(<App />)

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(useStore.getState().viewMode).toBe('editor')
    expect(screen.getByTestId('canvas-workspace-primary')).toBeVisible()
    expect(screen.getByTestId('inspector-panel')).toBeVisible()
    expect(screen.getByTestId('editor-no-flow-state')).toHaveTextContent('Select a flow to begin authoring.')

    act(() => {
      useStore.getState().registerProject('/tmp/project-shell')
    })

    expect(screen.getByTestId('top-nav-project-switcher')).toHaveTextContent('project-shell')
  })

  it('adds a project from the navbar picker and clears the active project', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/pick-directory')) {
          return new Response(JSON.stringify({ status: 'selected', directory_path: '/tmp/project-shell' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/register')) {
          return new Response(JSON.stringify({
            project_id: 'project-shell-1',
            project_path: '/tmp/project-shell',
            display_name: 'project-shell',
            is_favorite: false,
            active_conversation_id: null,
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main' }), {
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
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/api/flows')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/runs')) {
          return new Response(JSON.stringify({ runs: [] }), {
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

    render(<App />)

    await user.click(screen.getByTestId('top-nav-project-add-button'))

    await waitFor(() => {
      expect(useStore.getState().activeProjectPath).toBe('/tmp/project-shell')
    })
    expect(screen.getByTestId('top-nav-project-switcher')).toHaveTextContent('project-shell')

    await user.click(screen.getByTestId('top-nav-project-clear-button'))
    expect(useStore.getState().activeProjectPath).toBeNull()
  })

  it('propagates navbar project switches across Home, Execution, Triggers, and Runs', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().registerProject('/tmp/project-two')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
      useStore.getState().setViewMode('home')
    })
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'

      if (url.includes('/workspace/api/projects/metadata')) {
        return jsonResponse({ branch: 'main', commit: 'abcdef0' })
      }
      if (url.includes('/workspace/api/projects/state')) {
        const payload = init?.body ? JSON.parse(String(init.body)) as { project_path?: string } : {}
        return jsonResponse(buildProjectRecord(payload.project_path ?? '/tmp/project-one'))
      }
      if (url.includes('/workspace/api/projects/conversations')) {
        const projectPath = new URL(url, 'http://localhost').searchParams.get('project_path')
        if (projectPath === '/tmp/project-one') {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-project-one',
              projectPath: '/tmp/project-one',
              title: 'Thread one',
            }),
          ])
        }
        if (projectPath === '/tmp/project-two') {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-project-two',
              projectPath: '/tmp/project-two',
              title: 'Thread two',
            }),
          ])
        }
        return jsonResponse([])
      }
      if (url.includes('/workspace/api/conversations/') && method === 'GET') {
        return jsonResponse({ detail: 'Unknown conversation' }, { status: 404 })
      }
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([])
      }
      if (url.includes('/workspace/api/projects')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/status')) {
        return jsonResponse({ status: 'idle', last_run_id: null })
      }
      if (url.includes('/attractor/api/flows')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'review-one.dot', projectPath: '/tmp/project-one', runId: 'run-one' })],
        })
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-two')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'review-two.dot', projectPath: '/tmp/project-two', runId: 'run-two' })],
        })
      }
      if (url.includes('/attractor/runs')) {
        return jsonResponse({ runs: [] })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('Thread one')).toBeVisible()
    })
    await user.click(screen.getByTestId('top-nav-project-switcher'))
    await user.click(await screen.findByText('project-two'))

    await waitFor(() => {
      expect(useStore.getState().activeProjectPath).toBe('/tmp/project-two')
    })
    await waitFor(() => {
      expect(screen.getByText('Thread two')).toBeVisible()
    })
    expect(screen.queryByText('Thread one')).not.toBeInTheDocument()
    expect(screen.getByTestId('top-nav-project-switcher')).toHaveTextContent('project-two')
    expect(screen.getByTestId('top-nav-project-switcher')).not.toHaveTextContent('/tmp/project-two')

    await user.click(screen.getByTestId('nav-mode-execution'))
    expect(await screen.findByTestId('execution-project-context-chip')).toHaveTextContent('project-two')
    expect(screen.getByTestId('execution-no-flow-state')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-triggers'))
    expect(await screen.findByTestId('triggers-project-context-chip')).toHaveTextContent('project-two')
    expect(screen.getByLabelText('Execution Target')).toHaveValue('active')
    expect(screen.getByText('Uses the current active project: /tmp/project-two')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-runs'))
    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(([request]) =>
          resolveRequestUrl(request as RequestInfo | URL).includes('/attractor/runs?project_path=%2Ftmp%2Fproject-two'),
        ),
      ).toBe(true)
    })
    expect(await screen.findByTestId('runs-project-context-chip')).toHaveTextContent('project-two')
    expect(screen.getByText('Run history for the active project.')).toBeVisible()
    expect(screen.getByText('review-two.dot')).toBeVisible()
  })

  it('removes the active project from the navbar after confirmation', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-remove')
      useStore.getState().setActiveProjectPath('/tmp/project-remove')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects?project_path=') && init?.method === 'DELETE') {
          return new Response(JSON.stringify({
            status: 'deleted',
            project_id: 'project-remove',
            project_path: '/tmp/project-remove',
            display_name: 'project-remove',
          }), {
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
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/api/flows')) {
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

    render(<App />)

    await user.click(screen.getByTestId('top-nav-project-remove-button'))
    await user.click(screen.getByTestId('shared-dialog-confirm'))

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/project-remove']).toBeUndefined()
    })
    expect(useStore.getState().activeProjectPath).toBeNull()
  })

  it('returns to the active-project run scope after opening a run from all-project history', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = resolveRequestUrl(input)
      if (url.includes('/attractor/status')) {
        return jsonResponse({ status: 'idle', last_run_id: null })
      }
      if (url.includes('/attractor/api/flows')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'review-one.dot', projectPath: '/tmp/project-one', runId: 'run-one' })],
        })
      }
      if (url.endsWith('/attractor/runs')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'review-two.dot', projectPath: '/tmp/project-two', runId: 'run-two' })],
        })
      }
      if (url.includes('/attractor/pipelines/run-two/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-two',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'review',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-two/context')) {
        return jsonResponse({
          pipeline_id: 'run-two',
          context: {},
        })
      }
      if (url.includes('/attractor/pipelines/run-two/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-two',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-two/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-two/questions')) {
        return jsonResponse({
          pipeline_id: 'run-two',
          questions: [],
        })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
      useStore.getState().setViewMode('runs')
    })

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('review-one.dot')).toBeVisible()
    })
    const initialActiveScopeFetchCount = fetchMock.mock.calls.filter(([request]) =>
      resolveRequestUrl(request as RequestInfo | URL).includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one'),
    ).length

    await user.click(screen.getByTestId('runs-scope-all-projects'))
    await waitFor(() => {
      expect(screen.getByText('review-two.dot')).toBeVisible()
    })
    const runTwoCard = screen.getByText('review-two.dot').closest('[data-testid="run-history-row"]')
    expect(runTwoCard).not.toBeNull()
    await user.click(runTwoCard!)

    await waitFor(() => {
      expect(useStore.getState().viewMode).toBe('runs')
    })
    expect(useStore.getState().selectedRunId).toBe('run-two')

    await user.click(screen.getByTestId('nav-mode-projects'))
    await user.click(screen.getByTestId('nav-mode-runs'))
    await waitFor(() => {
      const activeScopeFetchCount = fetchMock.mock.calls.filter(([request]) =>
        resolveRequestUrl(request as RequestInfo | URL).includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one'),
      ).length
      expect(activeScopeFetchCount).toBeGreaterThan(initialActiveScopeFetchCount)
    })
    expect(screen.getByText('Run history for the active project.')).toBeVisible()
    expect(screen.getByText('review-one.dot')).toBeVisible()
    expect(screen.queryByText('review-two.dot')).not.toBeInTheDocument()
  })

  it('lets the operator resize the editor sidebar without affecting execution width', async () => {
    const user = userEvent.setup()
    setViewportWidth(1366)
    render(<App />)

    await user.click(screen.getByTestId('nav-mode-editor'))

    const inspectorPanel = screen.getByTestId('inspector-panel') as HTMLElement
    const resizeHandle = screen.getByTestId('editor-sidebar-resize-handle')
    const editorWorkspaceLayout = screen.getByTestId('editor-workspace').firstElementChild as HTMLDivElement

    vi.spyOn(editorWorkspaceLayout, 'getBoundingClientRect').mockReturnValue({
      x: 0,
      y: 0,
      top: 0,
      right: 1200,
      bottom: 720,
      left: 0,
      width: 1200,
      height: 720,
      toJSON: () => ({}),
    } as DOMRect)

    expect(inspectorPanel.style.width).toBe(`${DEFAULT_EDITOR_SIDEBAR_WIDTH}px`)

    fireEvent.pointerDown(resizeHandle, { clientX: DEFAULT_EDITOR_SIDEBAR_WIDTH })
    fireEvent.pointerMove(window, { clientX: 360 })
    fireEvent.pointerUp(window)

    expect(useStore.getState().editorSidebarWidth).toBe(360)
    expect(inspectorPanel.style.width).toBe('360px')

    await user.click(screen.getByTestId('nav-mode-execution'))
    const executionFlowPanel = screen.getByTestId('execution-flow-panel')
    expect(executionFlowPanel.className).toContain('w-72')
    expect(executionFlowPanel).not.toHaveStyle({ width: '360px' })

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(screen.getByTestId('inspector-panel').style.width).toBe('360px')
  })

  it('keeps the shell mounted when selecting a flow in editor mode', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes(`/attractor/api/flows/${LINEAR_FLOW_NAME}`)) {
          return new Response(JSON.stringify({
            name: LINEAR_FLOW_NAME,
            content: [
              'digraph simple_linear {',
              '  graph [label="Simple Linear Workflow", goal="Inspect the repo."];',
              '  start [shape=Mdiamond, label="Start"];',
              '  plan [shape=box, label="Plan", prompt="Plan the work."];',
              '  done [shape=Msquare, label="Done"];',
              '  start -> plan;',
              '  plan -> done;',
              '}',
            ].join('\n'),
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/api/flows')) {
          return new Response(JSON.stringify([LINEAR_FLOW_NAME]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/preview')) {
          return new Response(JSON.stringify({
            status: 'ok',
            graph: {
              graph_attrs: {
                label: 'Simple Linear Workflow',
                goal: 'Inspect the repo.',
                model_stylesheet: null,
                default_max_retries: null,
                retry_target: null,
                fallback_retry_target: null,
                default_fidelity: null,
                'stack.child_dotfile': null,
                'stack.child_workdir': null,
                'tool.hooks.pre': null,
                'tool.hooks.post': null,
                ui_default_llm_model: 'gpt-5.3-codex-spark',
                ui_default_llm_provider: 'openai',
                ui_default_reasoning_effort: null,
                'spark.description': 'A minimal starter flow.',
                'spark.title': 'Simple Linear Workflow',
              },
              nodes: [
                {
                  id: 'start',
                  label: 'Start',
                  shape: 'Mdiamond',
                  prompt: null,
                  'tool.command': null,
                  'tool.hooks.pre': null,
                  'tool.hooks.post': null,
                  'tool.artifacts.paths': null,
                  'tool.artifacts.stdout': null,
                  'tool.artifacts.stderr': null,
                  join_policy: 'wait_all',
                  error_policy: 'continue',
                  max_parallel: 4,
                  type: null,
                  max_retries: null,
                  goal_gate: false,
                  retry_target: null,
                  fallback_retry_target: null,
                  fidelity: null,
                  thread_id: null,
                  class: null,
                  timeout: null,
                  llm_model: '',
                  llm_provider: '',
                  reasoning_effort: 'high',
                  auto_status: false,
                  allow_partial: false,
                  'manager.poll_interval': null,
                  'manager.max_cycles': null,
                  'manager.stop_condition': null,
                  'manager.actions': null,
                  'human.default_choice': null,
                },
                {
                  id: 'plan',
                  label: 'Plan',
                  shape: 'box',
                  prompt: 'Plan the work.',
                  'tool.command': null,
                  'tool.hooks.pre': null,
                  'tool.hooks.post': null,
                  'tool.artifacts.paths': null,
                  'tool.artifacts.stdout': null,
                  'tool.artifacts.stderr': null,
                  join_policy: 'wait_all',
                  error_policy: 'continue',
                  max_parallel: 4,
                  type: null,
                  max_retries: null,
                  goal_gate: false,
                  retry_target: null,
                  fallback_retry_target: null,
                  fidelity: null,
                  thread_id: null,
                  class: null,
                  timeout: null,
                  llm_model: '',
                  llm_provider: '',
                  reasoning_effort: 'high',
                  auto_status: false,
                  allow_partial: false,
                  'manager.poll_interval': null,
                  'manager.max_cycles': null,
                  'manager.stop_condition': null,
                  'manager.actions': null,
                  'human.default_choice': null,
                },
                {
                  id: 'done',
                  label: 'Done',
                  shape: 'Msquare',
                  prompt: null,
                  'tool.command': null,
                  'tool.hooks.pre': null,
                  'tool.hooks.post': null,
                  'tool.artifacts.paths': null,
                  'tool.artifacts.stdout': null,
                  'tool.artifacts.stderr': null,
                  join_policy: 'wait_all',
                  error_policy: 'continue',
                  max_parallel: 4,
                  type: null,
                  max_retries: null,
                  goal_gate: false,
                  retry_target: null,
                  fallback_retry_target: null,
                  fidelity: null,
                  thread_id: null,
                  class: null,
                  timeout: null,
                  llm_model: '',
                  llm_provider: '',
                  reasoning_effort: 'high',
                  auto_status: false,
                  allow_partial: false,
                  'manager.poll_interval': null,
                  'manager.max_cycles': null,
                  'manager.stop_condition': null,
                  'manager.actions': null,
                  'human.default_choice': null,
                },
              ],
              edges: [
                { from: 'start', to: 'plan', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
                { from: 'plan', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              ],
            },
            diagnostics: [],
            errors: [],
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes(`/workspace/api/flows/${LINEAR_FLOW_NAME}?surface=human`)) {
          return new Response(JSON.stringify({
            name: LINEAR_FLOW_NAME,
            title: 'Simple Linear Workflow',
            description: 'A minimal starter flow.',
            launch_policy: null,
            effective_launch_policy: 'disabled',
            graph_label: 'Simple Linear Workflow',
            graph_goal: 'Inspect the repo.',
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/runs')) {
          return new Response(JSON.stringify({ runs: [] }), {
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

    render(<App />)
    await user.click(screen.getByTestId('nav-mode-editor'))
    const editorFlowTree = await screen.findByTestId('editor-flow-tree')
    await user.click(within(editorFlowTree).getByRole('button', { name: LINEAR_FLOW_NAME }))

    await waitFor(() => {
      expect(screen.getByTestId('app-shell')).toBeVisible()
      expect(screen.getByTestId('top-nav')).toBeVisible()
      expect(screen.getByTestId('inspector-panel')).toBeVisible()
      expect(screen.getByTestId('flow-browser-panel')).toBeVisible()
      expect(screen.getByTestId('graph-inspector-panel')).toBeVisible()
    })
  })

  it('renders the flow selector as a directory tree and opens nested editor flows', async () => {
    const user = userEvent.setup()
    let sawNestedAttractorFlowRequest = false
    let sawNestedWorkspaceFlowRequest = false

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes(`/attractor/api/flows/${NESTED_LINEAR_FLOW_NAME}`)) {
          sawNestedAttractorFlowRequest = true
          return new Response(JSON.stringify({
            name: NESTED_LINEAR_FLOW_NAME,
            content: [
              'digraph simple_linear {',
              '  graph [label="Team Review Flow", goal="Inspect nested flow loading."];',
              '  start [shape=Mdiamond, label="Start"];',
              '  plan [shape=box, label="Plan", prompt="Plan the work."];',
              '  done [shape=Msquare, label="Done"];',
              '  start -> plan;',
              '  plan -> done;',
              '}',
            ].join('\n'),
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/api/flows')) {
          return new Response(JSON.stringify([NESTED_LINEAR_FLOW_NAME]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/preview')) {
          return new Response(JSON.stringify({
            status: 'ok',
            graph: {
              graph_attrs: {
                label: 'Team Review Flow',
                goal: 'Inspect nested flow loading.',
                'spark.title': 'Team Review Flow',
              },
              nodes: [
                { id: 'start', label: 'Start', shape: 'Mdiamond' },
                { id: 'plan', label: 'Plan', shape: 'box', prompt: 'Plan the work.' },
                { id: 'done', label: 'Done', shape: 'Msquare' },
              ],
              edges: [
                { from: 'start', to: 'plan', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
                { from: 'plan', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              ],
            },
            diagnostics: [],
            errors: [],
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes(`/workspace/api/flows/${NESTED_LINEAR_FLOW_NAME}?surface=human`)) {
          sawNestedWorkspaceFlowRequest = true
          return new Response(JSON.stringify({
            name: NESTED_LINEAR_FLOW_NAME,
            title: 'Team Review Flow',
            description: 'A nested flow used to verify tree-based selection.',
            launch_policy: null,
            effective_launch_policy: 'disabled',
            graph_label: 'Team Review Flow',
            graph_goal: 'Inspect nested flow loading.',
          }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/runs')) {
          return new Response(JSON.stringify({ runs: [] }), {
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

    render(<App />)
    await user.click(screen.getByTestId('nav-mode-editor'))
    const editorFlowTree = await screen.findByTestId('editor-flow-tree')

    expect(within(editorFlowTree).getByText('team')).toBeVisible()
    expect(within(editorFlowTree).getByText('review')).toBeVisible()

    await user.click(within(editorFlowTree).getByRole('button', { name: NESTED_LINEAR_FLOW_NAME }))

    await waitFor(() => {
      expect(screen.getByTestId('editor-mode-toggle')).toBeVisible()
      expect(sawNestedAttractorFlowRequest).toBe(true)
      expect(sawNestedWorkspaceFlowRequest).toBe(true)
    })
  })

  it('keeps editor and execution sessions isolated across tab switches', async () => {
    const user = userEvent.setup()
    installCanvasWorkspaceFetchMock()

    render(<App />)

    await user.click(screen.getByTestId('nav-mode-execution'))
    const executionFlowTree = await screen.findByTestId('execution-flow-tree')
    await user.click(within(executionFlowTree).getByRole('button', { name: REVIEW_FLOW_NAME }))
    expect(await screen.findByTestId('execution-launch-inputs')).toBeVisible()

    await user.type(
      screen.getByTestId('execution-launch-input-context.request.summary'),
      'Review the auth flow',
    )

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(screen.getByTestId('inspector-panel')).toBeVisible()
    expect(screen.getByTestId('editor-no-flow-state')).toHaveTextContent('Select a flow to begin authoring.')
    expect(screen.getByTestId('editor-workspace')).toBeVisible()
    expect(screen.queryByTestId('execution-workspace')).not.toBeInTheDocument()

    const editorFlowTree = await screen.findByTestId('editor-flow-tree')
    await user.click(within(editorFlowTree).getByRole('button', { name: LINEAR_FLOW_NAME }))
    await waitFor(() => {
      expect(screen.getByTestId('editor-mode-toggle')).toBeVisible()
    })
    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const rawDotEditor = await screen.findByTestId('raw-dot-editor')
    await user.type(rawDotEditor, '\n// editor draft note')

    await user.click(screen.getByTestId('nav-mode-execution'))
    expect(await screen.findByTestId('execution-launch-flow-name')).toHaveTextContent(REVIEW_FLOW_NAME)
    expect(screen.getByTestId('execution-launch-input-context.request.summary')).toHaveValue('')

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect((await screen.findByTestId('raw-dot-editor') as HTMLTextAreaElement).value).toContain('// editor draft note')
  })

  it('keeps execution as a launch surface with the primary action inside the launch panel', async () => {
    const user = userEvent.setup()
    installCanvasWorkspaceFetchMock()

    render(<App />)

    await user.click(screen.getByTestId('nav-mode-execution'))
    const executionFlowTree = await screen.findByTestId('execution-flow-tree')
    await user.click(within(executionFlowTree).getByRole('button', { name: REVIEW_FLOW_NAME }))

    const executionWorkspace = await screen.findByTestId('execution-workspace')
    const executionLaunchPanel = screen.getByTestId('execution-launch-panel')
    const executionPrimaryAction = screen.getByTestId('execution-launch-primary-action')

    expect(executionWorkspace).toHaveAttribute('data-responsive-layout', 'split')
    expect(executionLaunchPanel).toContainElement(executionPrimaryAction)
    expect(screen.queryByTestId('execution-canvas-panel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('execution-footer-controls')).not.toBeInTheDocument()
  })
})
