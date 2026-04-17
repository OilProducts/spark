import App from '@/App'
import { useRunJournalStore } from '@/features/runs/state/runJournalStore'
import { useStore } from '@/store'
import { createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const DEFAULT_EDITOR_SIDEBAR_WIDTH = 288
const LINEAR_FLOW_NAME = 'test-linear.dot'
const REVIEW_FLOW_NAME = 'test-review-loop.dot'
const NESTED_LINEAR_FLOW_NAME = 'team/review/test-linear.dot'

class MockEventSource {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 2
  static readonly instances: MockEventSource[] = []
  readonly url: string
  readyState = MockEventSource.OPEN
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent<string>) => void) | null = null
  onerror: ((event: Event) => void) | null = null

  constructor(url: string | URL) {
    this.url = String(url)
    MockEventSource.instances.push(this)
  }

  close() {
    this.readyState = MockEventSource.CLOSED
  }

  emitMessage(payload: unknown) {
    this.onmessage?.(new MessageEvent('message', {
      data: typeof payload === 'string' ? payload : JSON.stringify(payload),
    }))
  }
}

const findEventSource = (pattern: string) =>
  MockEventSource.instances.find((eventSource) => eventSource.url.includes(pattern)) ?? null

const resetMockEventSources = () => {
  MockEventSource.instances.length = 0
}

const stableTimelineEvent = <T extends Record<string, unknown>>(sequence: number, payload: T) => ({
  ...payload,
  sequence,
  emitted_at: `2026-04-06T12:00:${String(sequence).padStart(2, '0')}Z`,
})

const setViewportWidth = (width: number) => {
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: width,
  })
  window.dispatchEvent(new Event('resize'))
}

const mockSidebarStackRect = (node: HTMLDivElement, initialHeight: number) => {
  let height = initialHeight
  vi.spyOn(node, 'getBoundingClientRect').mockImplementation(() => ({
    x: 0,
    y: 0,
    top: 0,
    right: 320,
    bottom: height,
    left: 0,
    width: 320,
    height,
    toJSON: () => ({}),
  } as DOMRect))
}

const resetAppShellState = () => {
  useRunJournalStore.setState({ byRunId: {} })
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
    runtimeStatus: 'idle',
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    humanGate: null,
    nodeStatuses: {},
    selectedNodeId: null,
    selectedEdgeId: null,
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
    graphAttrs: {},
    graphAttrErrors: {},
    editorSidebarWidth: DEFAULT_EDITOR_SIDEBAR_WIDTH,
    editorMode: 'structured',
    rawDotDraft: '',
    rawHandoffError: null,
    editorGraphSettingsPanelOpenByFlow: {},
    editorShowAdvancedGraphAttrsByFlow: {},
    editorLaunchInputDraftsByFlow: {},
    editorLaunchInputDraftErrorByFlow: {},
    editorNodeInspectorSessionsByNodeId: {},
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    runsListSession: {
      scopeMode: 'active',
      selectedRunIdByScopeKey: {},
      status: 'idle',
      error: null,
      runs: [],
      streamStatus: 'idle',
      streamError: null,
    },
    runDetailSessionsByRunId: {},
    triggersSession: {
      status: 'idle',
      error: null,
      triggers: [],
      selectedTriggerId: null,
      scopeFilter: 'all',
      revealedWebhookSecrets: {},
      newTriggerDraft: {
        form: createEmptyTriggerForm(null),
        targetBehavior: 'default',
      },
      editTriggerDraftsByTriggerId: {},
    },
    executionLaunchInputValues: {},
    executionLaunchError: null,
    executionLastLaunchFailure: null,
    executionRunStartGitPolicyWarning: null,
    executionCollapsedLaunchInputsByFlow: {},
    executionGraphCollapsed: false,
    executionOpenRunsAfterLaunch: false,
    executionLaunchSuccessRunId: null,
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

const createDeferred = <T,>() => {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((innerResolve, innerReject) => {
    resolve = innerResolve
    reject = innerReject
  })
  return { promise, resolve, reject }
}

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

const buildConversationSnapshot = ({
  conversationId,
  projectPath,
  title,
  turns = [],
  segments = [],
}: {
  conversationId: string
  projectPath: string
  title: string
  turns?: Array<{
    id: string
    role: 'user' | 'assistant'
    status: 'pending' | 'streaming' | 'complete' | 'failed'
    content: string
    timestamp: string
  }>
  segments?: Array<Record<string, unknown>>
}) => ({
  schema_version: 4,
  conversation_id: conversationId,
  conversation_handle: `${conversationId}-handle`,
  project_path: projectPath,
  title,
  created_at: '2026-03-25T12:00:00Z',
  updated_at: '2026-03-25T12:05:00Z',
  turns,
  segments,
  event_log: [],

  flow_run_requests: [],
  flow_launches: [],


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

const buildTriggerRecord = ({
  id,
  name,
  projectPath = null,
  protectedTrigger = false,
}: {
  id: string
  name: string
  projectPath?: string | null
  protectedTrigger?: boolean
}) => ({
  id,
  name,
  enabled: true,
  protected: protectedTrigger,
  source_type: 'schedule',
  created_at: '2026-03-22T00:00:00Z',
  updated_at: '2026-03-22T00:00:00Z',
  action: {
    flow_name: 'test-planning.dot',
    project_path: projectPath,
    static_context: {},
  },
  source: { kind: 'interval', interval_seconds: 300 },
  state: {
    last_fired_at: null,
    last_result: null,
    last_error: null,
    next_run_at: '2026-03-22T00:05:00Z',
    recent_history: [],
  },
  webhook_secret: null,
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
        return new Response(JSON.stringify({ status: 'idle' }), {
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
    resetMockEventSources()
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
          return new Response(JSON.stringify({ status: 'idle' }), {
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
    resetMockEventSources()
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

  it('adds a project from the remote browser modal and clears the active project', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/browse')) {
          return new Response(JSON.stringify({
            current_path: '/tmp/project-shell',
            parent_path: '/tmp',
            entries: [],
          }), {
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
          return new Response(JSON.stringify({ status: 'idle' }), {
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
    expect(screen.queryByTestId('project-directory-picker-input')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('top-nav-project-add-button'))
    expect(await screen.findByTestId('project-browser-dialog')).toBeVisible()
    expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/tmp/project-shell')
    await user.click(screen.getByTestId('project-browser-select-button'))

    await waitFor(() => {
      expect(useStore.getState().activeProjectPath).toBe('/tmp/project-shell')
    })
    expect(screen.getByTestId('top-nav-project-switcher')).toHaveTextContent('project-shell')

    await user.click(screen.getByTestId('top-nav-project-clear-button'))
    expect(useStore.getState().activeProjectPath).toBeNull()
  })

  it('navigates the remote browser modal and surfaces browse failures', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = new URL(resolveRequestUrl(input), 'http://localhost')
        if (url.pathname === '/workspace/api/projects/browse') {
          const requestedPath = url.searchParams.get('path')
          if (requestedPath === null) {
            return jsonResponse({
              current_path: '/home/spark',
              parent_path: '/home',
              entries: [
                { name: 'broken', path: '/home/spark/broken', is_dir: true },
                { name: 'repo', path: '/home/spark/repo', is_dir: true },
              ],
            })
          }
          if (requestedPath === '/home/spark/broken') {
            return jsonResponse({ detail: 'Browse backend unavailable.' }, { status: 503 })
          }
          if (requestedPath === '/home/spark/repo') {
            return jsonResponse({
              current_path: '/home/spark/repo',
              parent_path: '/home/spark',
              entries: [],
            })
          }
          if (requestedPath === '/home') {
            return jsonResponse({
              current_path: '/home',
              parent_path: '/',
              entries: [{ name: 'spark', path: '/home/spark', is_dir: true }],
            })
          }
          if (requestedPath === '/') {
            return jsonResponse({
              current_path: '/',
              parent_path: null,
              entries: [{ name: 'home', path: '/home', is_dir: true }],
            })
          }
        }
        if (url.pathname === '/workspace/api/projects') {
          return jsonResponse([])
        }
        if (url.pathname === '/attractor/status') {
          return jsonResponse({ status: 'idle' })
        }
        if (url.pathname === '/attractor/api/flows') {
          return jsonResponse([])
        }
        if (url.pathname === '/attractor/runs') {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await user.click(screen.getByTestId('top-nav-project-add-button'))
    expect(await screen.findByTestId('project-browser-dialog')).toBeVisible()
    expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/home/spark')

    await user.click(screen.getByTestId('project-browser-entry-broken'))
    expect(await screen.findByTestId('project-browser-error')).toHaveTextContent('Browse backend unavailable.')

    await user.click(screen.getByTestId('project-browser-entry-repo'))
    await waitFor(() => {
      expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/home/spark/repo')
    })

    await user.click(screen.getByTestId('project-browser-parent-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/home/spark')
    })

    await user.click(screen.getByRole('button', { name: 'Root' }))
    await waitFor(() => {
      expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/')
    })

    await user.click(screen.getByTestId('project-browser-entry-home'))
    await waitFor(() => {
      expect(screen.getByTestId('project-browser-current-path')).toHaveTextContent('/home')
    })
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
        return jsonResponse({ status: 'idle' })
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

  it('preserves the Home thread session across tab switches', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-home')
      useStore.getState().setActiveProjectPath('/tmp/project-home')
      useStore.getState().setViewMode('home')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-home-one',
              projectPath: '/tmp/project-home',
              title: 'Thread home',
            }),
          ])
        }
        if (url.includes('/workspace/api/conversations/conversation-home-one') && !url.includes('/events')) {
          return jsonResponse(buildConversationSnapshot({
            conversationId: 'conversation-home-one',
            projectPath: '/tmp/project-home',
            title: 'Thread home',
            turns: [
              {
                id: 'turn-user-1',
                role: 'user',
                status: 'complete',
                content: 'Initial request',
                timestamp: '2026-03-25T12:00:00Z',
              },
              {
                id: 'turn-assistant-1',
                role: 'assistant',
                status: 'complete',
                content: 'Existing assistant reply',
                timestamp: '2026-03-25T12:01:00Z',
              },
            ],
          }))
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            {
              ...buildProjectRecord('/tmp/project-home'),
              active_conversation_id: 'conversation-home-one',
            },
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Open thread Thread home' })).toBeVisible()
      expect(screen.getByText('Existing assistant reply')).toBeVisible()
    })

    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Need follow-up')

    await user.click(screen.getByTestId('nav-mode-runs'))
    await user.click(screen.getByTestId('nav-mode-projects'))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Open thread Thread home' })).toBeVisible()
    })
    expect(screen.getByTestId('project-ai-conversation-input')).toHaveValue('Need follow-up')
    expect(screen.queryByTestId('project-thread-list-loading')).not.toBeInTheDocument()
  })

  it('preserves Home layout, expansion, and scroll session state across tab switches', async () => {
    const user = userEvent.setup()
    setViewportWidth(1366)

    act(() => {
      useStore.getState().registerProject('/tmp/project-home-restore')
      useStore.getState().setActiveProjectPath('/tmp/project-home-restore')
      useStore.getState().setViewMode('home')
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-home-restore',
              projectPath: '/tmp/project-home-restore',
              title: 'Thread restore',
            }),
          ])
        }
        if (url.includes('/workspace/api/conversations/conversation-home-restore') && !url.includes('/events')) {
          return jsonResponse(buildConversationSnapshot({
            conversationId: 'conversation-home-restore',
            projectPath: '/tmp/project-home-restore',
            title: 'Thread restore',
            turns: [
              {
                id: 'turn-user-1',
                role: 'user',
                status: 'complete',
                content: 'Run ls and summarize it.',
                timestamp: '2026-03-25T12:00:00Z',
              },
              {
                id: 'turn-assistant-1',
                role: 'assistant',
                status: 'complete',
                content: 'Summary after tools.',
                timestamp: '2026-03-25T12:01:00Z',
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
                timestamp: '2026-03-25T12:00:30Z',
                updated_at: '2026-03-25T12:00:30Z',
                completed_at: '2026-03-25T12:00:30Z',
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
                timestamp: '2026-03-25T12:01:00Z',
                updated_at: '2026-03-25T12:01:00Z',
                completed_at: '2026-03-25T12:01:00Z',
                content: 'Summary after tools.',
                artifact_id: null,
                error: null,
                tool_call: null,
                source: null,
              },
            ],
          }))
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            {
              ...buildProjectRecord('/tmp/project-home-restore'),
              active_conversation_id: 'conversation-home-restore',
            },
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await waitFor(() => {
      expect(screen.getByTestId('project-tool-call-toggle-tool-ls')).toBeVisible()
    })

    await user.click(screen.getByTestId('project-tool-call-toggle-tool-ls'))
    expect(within(screen.getByTestId('project-ai-conversation-history')).getByText(/AGENTS\.md/)).toBeVisible()

    const sidebarStack = screen.getByTestId('home-sidebar-stack')
    const sidebarPrimarySurface = screen.getByTestId('home-sidebar-primary-surface') as HTMLDivElement
    const resizeHandle = screen.getByTestId('home-sidebar-resize-handle')
    mockSidebarStackRect(sidebarStack as HTMLDivElement, 720)

    fireEvent.pointerDown(resizeHandle, { clientY: 240 })
    fireEvent.pointerMove(window, { clientY: 300 })
    fireEvent.pointerUp(window)

    await waitFor(() => {
      expect(sidebarPrimarySurface.style.height).toBe('380px')
    })

    const conversationBody = screen.getByTestId('project-ai-conversation-body') as HTMLDivElement
    let scrollTop = 120
    Object.defineProperty(conversationBody, 'clientHeight', {
      configurable: true,
      get: () => 200,
    })
    Object.defineProperty(conversationBody, 'scrollHeight', {
      configurable: true,
      get: () => 640,
    })
    Object.defineProperty(conversationBody, 'scrollTop', {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = value
      },
    })

    act(() => {
      fireEvent.scroll(conversationBody)
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-ai-conversation-jump-to-bottom')).toBeVisible()
    })

    expect(
      useStore.getState().homeProjectSessionsByPath['/tmp/project-home-restore']?.sidebarPrimarySplitRatio,
    ).toBeCloseTo(380 / (720 - 12), 5)
    expect(useStore.getState().homeConversationSessionsById['conversation-home-restore']?.expandedToolCalls).toMatchObject({
      'tool-ls': true,
    })
    expect(useStore.getState().homeConversationSessionsById['conversation-home-restore']?.scrollTop).toBe(120)

    await user.click(screen.getByTestId('nav-mode-runs'))
    await user.click(screen.getByTestId('nav-mode-projects'))

    await waitFor(() => {
      expect(within(screen.getByTestId('project-ai-conversation-history')).getByText(/AGENTS\.md/)).toBeVisible()
    })
    expect(sidebarPrimarySurface.style.height).toBe('380px')
    expect(screen.getByTestId('project-ai-conversation-jump-to-bottom')).toBeVisible()
    expect(scrollTop).toBe(120)
  })

  it('keeps Home conversation sync live while Home is hidden', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-home-live')
      useStore.getState().setActiveProjectPath('/tmp/project-home-live')
      useStore.getState().setViewMode('home')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-home-live',
              projectPath: '/tmp/project-home-live',
              title: 'Thread live',
            }),
          ])
        }
        if (url.includes('/workspace/api/conversations/conversation-home-live') && !url.includes('/events')) {
          return jsonResponse(buildConversationSnapshot({
            conversationId: 'conversation-home-live',
            projectPath: '/tmp/project-home-live',
            title: 'Thread live',
            turns: [
              {
                id: 'turn-user-1',
                role: 'user',
                status: 'complete',
                content: 'Initial request',
                timestamp: '2026-03-25T12:00:00Z',
              },
              {
                id: 'turn-assistant-1',
                role: 'assistant',
                status: 'streaming',
                content: 'Thinking...',
                timestamp: '2026-03-25T12:01:00Z',
              },
            ],
          }))
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            {
              ...buildProjectRecord('/tmp/project-home-live'),
              active_conversation_id: 'conversation-home-live',
            },
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await waitFor(() => {
      expect(
        within(screen.getByTestId('project-ai-conversation-history')).getByText('Thinking...'),
      ).toBeVisible()
    })

    const conversationEventSource = findEventSource('/workspace/api/conversations/conversation-home-live/events')
    expect(conversationEventSource).not.toBeNull()

    await user.click(screen.getByTestId('nav-mode-runs'))

    act(() => {
      conversationEventSource?.emitMessage({
        type: 'conversation_snapshot',
        state: buildConversationSnapshot({
          conversationId: 'conversation-home-live',
          projectPath: '/tmp/project-home-live',
          title: 'Thread live',
          turns: [
            {
              id: 'turn-user-1',
              role: 'user',
              status: 'complete',
              content: 'Initial request',
              timestamp: '2026-03-25T12:00:00Z',
            },
            {
              id: 'turn-assistant-1',
              role: 'assistant',
              status: 'complete',
              content: 'Finished while hidden',
              timestamp: '2026-03-25T12:01:30Z',
            },
          ],
        }),
      })
    })

    await user.click(screen.getByTestId('nav-mode-projects'))

    await waitFor(() => {
      expect(
        within(screen.getByTestId('project-ai-conversation-history')).getByText('Finished while hidden'),
      ).toBeVisible()
    })
    expect(screen.queryByTestId('project-thread-list-loading')).not.toBeInTheDocument()
  })

  it('keeps Home project sessions live across active-project changes', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-home-one')
      useStore.getState().registerProject('/tmp/project-home-two')
      useStore.getState().setActiveProjectPath('/tmp/project-home-one')
      useStore.getState().setViewMode('home')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations?project_path=%2Ftmp%2Fproject-home-one')) {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-home-one',
              projectPath: '/tmp/project-home-one',
              title: 'Thread one',
            }),
          ])
        }
        if (url.includes('/workspace/api/projects/conversations?project_path=%2Ftmp%2Fproject-home-two')) {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-home-two',
              projectPath: '/tmp/project-home-two',
              title: 'Thread two',
            }),
          ])
        }
        if (url.includes('/workspace/api/conversations/conversation-home-one') && !url.includes('/events')) {
          return jsonResponse(buildConversationSnapshot({
            conversationId: 'conversation-home-one',
            projectPath: '/tmp/project-home-one',
            title: 'Thread one',
            turns: [
              {
                id: 'turn-user-one',
                role: 'user',
                status: 'complete',
                content: 'Project one request',
                timestamp: '2026-03-25T12:00:00Z',
              },
              {
                id: 'turn-assistant-one',
                role: 'assistant',
                status: 'streaming',
                content: 'Thinking...',
                timestamp: '2026-03-25T12:01:00Z',
              },
            ],
          }))
        }
        if (url.includes('/workspace/api/conversations/conversation-home-two') && !url.includes('/events')) {
          return jsonResponse(buildConversationSnapshot({
            conversationId: 'conversation-home-two',
            projectPath: '/tmp/project-home-two',
            title: 'Thread two',
            turns: [
              {
                id: 'turn-user-two',
                role: 'user',
                status: 'complete',
                content: 'Project two request',
                timestamp: '2026-03-25T12:02:00Z',
              },
              {
                id: 'turn-assistant-two',
                role: 'assistant',
                status: 'complete',
                content: 'Project two ready',
                timestamp: '2026-03-25T12:03:00Z',
              },
            ],
          }))
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            {
              ...buildProjectRecord('/tmp/project-home-one'),
              active_conversation_id: 'conversation-home-one',
            },
            {
              ...buildProjectRecord('/tmp/project-home-two'),
              active_conversation_id: 'conversation-home-two',
            },
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await waitFor(() => {
      expect(
        within(screen.getByTestId('project-ai-conversation-history')).getByText('Thinking...'),
      ).toBeVisible()
    })

    const projectOneEventSource = findEventSource('/workspace/api/conversations/conversation-home-one/events')
    expect(projectOneEventSource).not.toBeNull()

    await user.click(screen.getByTestId('top-nav-project-switcher'))
    await user.click(await screen.findByText('project-home-two'))

    await waitFor(() => {
      expect(useStore.getState().activeProjectPath).toBe('/tmp/project-home-two')
    })
    await waitFor(() => {
      expect(
        within(screen.getByTestId('project-ai-conversation-history')).getByText('Project two ready'),
      ).toBeVisible()
    })
    expect(projectOneEventSource?.readyState).toBe(MockEventSource.OPEN)

    act(() => {
      projectOneEventSource?.emitMessage({
        type: 'conversation_snapshot',
        state: buildConversationSnapshot({
          conversationId: 'conversation-home-one',
          projectPath: '/tmp/project-home-one',
          title: 'Thread one',
          turns: [
            {
              id: 'turn-user-one',
              role: 'user',
              status: 'complete',
              content: 'Project one request',
              timestamp: '2026-03-25T12:00:00Z',
            },
            {
              id: 'turn-assistant-one',
              role: 'assistant',
              status: 'complete',
              content: 'Finished while hidden on other project',
              timestamp: '2026-03-25T12:04:00Z',
            },
          ],
        }),
      })
    })

    await user.click(screen.getByTestId('top-nav-project-switcher'))
    await user.click(await screen.findByText('project-home-one'))

    await waitFor(() => {
      expect(useStore.getState().activeProjectPath).toBe('/tmp/project-home-one')
    })
    await waitFor(() => {
      expect(
        within(screen.getByTestId('project-ai-conversation-history')).getByText('Finished while hidden on other project'),
      ).toBeVisible()
    })
    expect(screen.queryByTestId('project-thread-list-loading')).not.toBeInTheDocument()
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
          return new Response(JSON.stringify({ status: 'idle' }), {
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

  it('preserves trigger selection and unsaved drafts across tab switches', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-trigger')
      useStore.getState().setActiveProjectPath('/tmp/project-trigger')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.endsWith('/workspace/api/triggers')) {
          return jsonResponse([
            buildTriggerRecord({
              id: 'trigger-protected',
              name: 'Protected route',
              projectPath: '/tmp/project-trigger',
              protectedTrigger: true,
            }),
            buildTriggerRecord({
              id: 'trigger-custom',
              name: 'Custom route',
              projectPath: '/tmp/project-trigger',
            }),
          ])
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return jsonResponse([])
        }
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([buildProjectRecord('/tmp/project-trigger')])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await user.click(screen.getByTestId('nav-mode-triggers'))
    await waitFor(() => {
      expect(screen.getByText('Custom route')).toBeVisible()
    })

    await user.click(screen.getByText('Custom route'))
    const nameInputs = screen.getAllByLabelText('Name')
    await user.type(nameInputs[0], ' Draft create')
    await user.clear(nameInputs[1])
    await user.type(nameInputs[1], 'Custom route edited')

    await user.click(screen.getByTestId('nav-mode-projects'))
    await user.click(screen.getByTestId('nav-mode-triggers'))

    const restoredNameInputs = screen.getAllByLabelText('Name')
    expect(restoredNameInputs[0]).toHaveValue(' Draft create')
    expect(restoredNameInputs[1]).toHaveValue('Custom route edited')
  })

  it('preserves trigger selection and drafts across active-project changes', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/project-trigger-one')
      useStore.getState().registerProject('/tmp/project-trigger-two')
      useStore.getState().setActiveProjectPath('/tmp/project-trigger-one')
      useStore.getState().setViewMode('triggers')
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.endsWith('/workspace/api/triggers')) {
          return jsonResponse([
            buildTriggerRecord({
              id: 'trigger-shared',
              name: 'Shared route',
              projectPath: '/tmp/project-trigger-one',
            }),
          ])
        }
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return jsonResponse([])
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            buildProjectRecord('/tmp/project-trigger-one'),
            buildProjectRecord('/tmp/project-trigger-two'),
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('Shared route')).toBeVisible()
    })

    await user.click(screen.getByText('Shared route'))
    const initialNameInputs = screen.getAllByLabelText('Name')
    await user.type(initialNameInputs[0], ' Draft create')
    await user.clear(initialNameInputs[1])
    await user.type(initialNameInputs[1], 'Shared route edited')

    act(() => {
      useStore.getState().setActiveProjectPath('/tmp/project-trigger-two')
    })

    await waitFor(() => {
      const restoredNameInputs = screen.getAllByLabelText('Name')
      expect(restoredNameInputs[0]).toHaveValue(' Draft create')
      expect(restoredNameInputs[1]).toHaveValue('Shared route edited')
    })
  })

  it('renders loading states before authoritative empty states for Home, Runs, and Triggers', async () => {
    const user = userEvent.setup()
    const homeThreadsDeferred = createDeferred<Response>()
    const runsDeferred = createDeferred<Response>()
    const triggersDeferred = createDeferred<Response>()

    act(() => {
      useStore.getState().registerProject('/tmp/project-loading')
      useStore.getState().setActiveProjectPath('/tmp/project-loading')
      useStore.getState().setViewMode('home')
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({ branch: 'main', commit: 'abcdef0' })
        }
        if (url.includes('/workspace/api/projects/conversations')) {
          return homeThreadsDeferred.promise
        }
        if (url.includes('/workspace/api/projects')) {
          return jsonResponse([
            {
              ...buildProjectRecord('/tmp/project-loading'),
              active_conversation_id: null,
            },
          ])
        }
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-loading')) {
          return runsDeferred.promise
        }
        if (url.endsWith('/workspace/api/triggers')) {
          return triggersDeferred.promise
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [] })
        }
        return jsonResponse({})
      }),
    )

    render(<App />)

    expect(await screen.findByTestId('project-thread-list-loading')).toBeVisible()
    homeThreadsDeferred.resolve(jsonResponse([]))
    await waitFor(() => {
      expect(screen.getByText('No threads for this project yet.')).toBeVisible()
    })

    await user.click(screen.getByTestId('nav-mode-runs'))
    expect(await screen.findByTestId('run-list-loading')).toBeVisible()
    runsDeferred.resolve(jsonResponse({ runs: [] }))
    await waitFor(() => {
      expect(screen.getByText('No runs for the active project yet.')).toBeVisible()
    })

    await user.click(screen.getByTestId('nav-mode-triggers'))
    expect(await screen.findByTestId('triggers-system-list-loading')).toBeVisible()
    triggersDeferred.resolve(jsonResponse([]))
    await waitFor(() => {
      expect(screen.getByText('No protected triggers in this scope.')).toBeVisible()
      expect(screen.getByText('No custom triggers in this scope yet.')).toBeVisible()
    })
  })

  it('preserves runs scope and selected run across tab switches', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = resolveRequestUrl(input)
      if (url.includes('/attractor/status')) {
        return jsonResponse({ status: 'idle' })
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

    await user.click(screen.getByTestId('runs-scope-all-projects'))
    await waitFor(() => {
      expect(screen.getByText('review-two.dot')).toBeVisible()
    })
    const runTwoCard = within(screen.getByTestId('run-list-scroll-region'))
      .getByText('review-two.dot')
      .closest('[data-testid="run-history-row"]')
    expect(runTwoCard).not.toBeNull()
    await user.click(runTwoCard!)

    await waitFor(() => {
      expect(useStore.getState().viewMode).toBe('runs')
    })
    expect(useStore.getState().selectedRunId).toBe('run-two')

    await user.click(screen.getByTestId('nav-mode-projects'))
    await user.click(screen.getByTestId('nav-mode-runs'))
    await waitFor(() => {
      expect(screen.getByTestId('run-summary-flow-name')).toHaveTextContent('review-two.dot')
    })
    expect(screen.getByText('Run history across all projects.')).toBeVisible()
    expect(screen.getByTestId('run-summary-flow-name')).toHaveTextContent('review-two.dot')
    expect(useStore.getState().selectedRunId).toBe('run-two')
  })

  it('preserves run-local inspection session state across tab switches', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/attractor/status')) {
          return jsonResponse({ status: 'idle' })
        }
        if (url.includes('/attractor/api/flows')) {
          return jsonResponse([])
        }
        if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-runs-session')) {
          return jsonResponse({
            runs: [
              buildRunRecord({
                flowName: 'review-session.dot',
                projectPath: '/tmp/project-runs-session',
                runId: 'run-session',
              }),
            ],
          })
        }
        if (url.endsWith('/attractor/pipelines/run-session')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            run_id: 'run-session',
            status: 'completed',
            outcome: 'success',
            outcome_reason_code: null,
            outcome_reason_message: null,
            flow_name: 'review-session.dot',
            working_directory: '/tmp/project-runs-session',
            project_path: '/tmp/project-runs-session',
            git_branch: 'main',
            git_commit: 'abcdef0',
            spec_id: null,
            plan_id: null,
            model: 'gpt-5.3-codex-spark',
            started_at: '2026-03-25T12:00:00Z',
            ended_at: '2026-03-25T12:05:00Z',
            last_error: '',
            token_usage: 1234,
            current_node: 'review',
            completed_nodes: ['prepare', 'review'],
            progress: {
              current_node: 'review',
              completed_nodes: ['prepare', 'review'],
            },
            continued_from_run_id: null,
            continued_from_node: null,
            continued_from_flow_mode: null,
            continued_from_flow_name: null,
          })
        }
        if (url.includes('/attractor/pipelines/run-session/checkpoint')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: 'review',
            },
          })
        }
        if (url.includes('/attractor/pipelines/run-session/context')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            context: {
              alpha: 'first',
              beta: 'second',
            },
          })
        }
        if (url.includes('/attractor/pipelines/run-session/artifacts/logs/summary.txt')) {
          return new Response('artifact preview contents', {
            status: 200,
            headers: { 'Content-Type': 'text/plain' },
          })
        }
        if (url.includes('/attractor/pipelines/run-session/artifacts')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            artifacts: [
              {
                path: 'logs/summary.txt',
                size_bytes: 42,
                media_type: 'text/plain',
                viewable: true,
              },
            ],
          })
        }
        if (url.includes('/attractor/pipelines/run-session/graph-preview')) {
          return jsonResponse({
            status: 'ok',
            graph: {
              graph_attrs: {},
              nodes: [
                { id: 'start', label: 'Start', shape: 'Mdiamond' },
                { id: 'review', label: 'Review', shape: 'box' },
                { id: 'done', label: 'Done', shape: 'Msquare' },
              ],
              edges: [
                { from: 'start', to: 'review', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
                { from: 'review', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              ],
            },
            diagnostics: [],
            errors: [],
          })
        }
        if (url.includes('/attractor/pipelines/run-session/questions')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            questions: [
              {
                question_id: 'gate-freeform',
                question_type: 'FREEFORM',
                node_id: 'review_gate',
                prompt: 'Need another review pass?',
              },
            ],
          })
        }
        if (url.includes('/attractor/pipelines/run-session/journal')) {
          return jsonResponse({
            pipeline_id: 'run-session',
            entries: [],
            oldest_sequence: null,
            newest_sequence: null,
            has_older: false,
          })
        }
        return jsonResponse({})
      }),
    )

    act(() => {
      useStore.getState().registerProject('/tmp/project-runs-session')
      useStore.getState().setActiveProjectPath('/tmp/project-runs-session')
      useStore.getState().setViewMode('runs')
    })

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('review-session.dot')).toBeVisible()
    })

    const runRow = within(screen.getByTestId('run-list-scroll-region'))
      .getByText('review-session.dot')
      .closest('[data-testid="run-history-row"]')
    expect(runRow).not.toBeNull()
    await user.click(runRow!)

    await waitFor(() => {
      expect(screen.getByTestId('run-advanced-panel')).toBeVisible()
      expect(screen.getByTestId('run-pending-human-gate-freeform-input-gate-freeform')).toBeVisible()
    })
    expect(screen.queryByTestId('run-context-search-input')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('run-advanced-toggle-button'))

    await waitFor(() => {
      expect(screen.getByTestId('run-context-search-input')).toBeVisible()
      expect(screen.getByTestId('run-artifact-view-button')).toBeVisible()
    })

    await waitFor(() => {
      expect(
        MockEventSource.instances.some((source) => source.url.includes('/attractor/pipelines/run-session/events')),
      ).toBe(true)
    })
    act(() => {
      MockEventSource.instances
        .filter((source) => source.url.includes('/attractor/pipelines/run-session/events'))
        .forEach((source) => source.emitMessage(stableTimelineEvent(1, {
          type: 'StageStarted',
          node_id: 'review',
          index: 2,
        })))
    })

    await waitFor(() => {
      expect(
        within(screen.getByTestId('run-event-timeline-panel')).getByText('Stage review started'),
      ).toBeVisible()
    })
    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-filter-type')).toBeVisible()
    })

    await user.selectOptions(screen.getByTestId('run-event-timeline-filter-type'), 'StageStarted')
    await user.type(screen.getByTestId('run-event-timeline-filter-node-stage'), 'review')
    await user.selectOptions(screen.getByTestId('run-event-timeline-filter-category'), 'stage')
    await user.selectOptions(screen.getByTestId('run-event-timeline-filter-severity'), 'info')
    await user.clear(screen.getByTestId('run-context-search-input'))
    await user.type(screen.getByTestId('run-context-search-input'), 'alpha')
    await user.click(screen.getByTestId('run-artifact-view-button'))

    await waitFor(() => {
      expect(screen.getByTestId('run-artifact-viewer-payload')).toHaveTextContent('artifact preview contents')
    })

    await user.type(
      screen.getByTestId('run-pending-human-gate-freeform-input-gate-freeform'),
      'Need another pass',
    )

    expect(useStore.getState().runDetailSessionsByRunId['run-session']).toMatchObject({
      timelineTypeFilter: 'StageStarted',
      timelineNodeStageFilter: 'review',
      timelineCategoryFilter: 'stage',
      timelineSeverityFilter: 'info',
      contextSearchQuery: 'alpha',
      selectedArtifactPath: 'logs/summary.txt',
      freeformAnswersByGateId: {
        'gate-freeform': 'Need another pass',
      },
    })

    await user.click(screen.getByTestId('nav-mode-projects'))
    await user.click(screen.getByTestId('nav-mode-runs'))

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-flow-name')).toHaveTextContent('review-session.dot')
    })
    expect(screen.getByTestId('run-event-timeline-filter-type')).toHaveValue('StageStarted')
    expect(screen.getByTestId('run-event-timeline-filter-node-stage')).toHaveValue('review')
    expect(screen.getByTestId('run-event-timeline-filter-category')).toHaveValue('stage')
    expect(screen.getByTestId('run-event-timeline-filter-severity')).toHaveValue('info')
    expect(screen.getByTestId('run-context-search-input')).toHaveValue('alpha')
    expect(screen.getByTestId('run-artifact-viewer')).toHaveTextContent('Preview: logs/summary.txt')
    expect(screen.getByTestId('run-artifact-viewer-payload')).toHaveTextContent('artifact preview contents')
    expect(screen.getByTestId('run-pending-human-gate-freeform-input-gate-freeform')).toHaveValue('Need another pass')
  })

  it('keeps selected run timeline sync live while Runs is hidden', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = resolveRequestUrl(input)
      if (url.includes('/workspace/api/projects')) {
        return jsonResponse([buildProjectRecord('/tmp/project-hidden-runs')])
      }
      if (url.includes('/workspace/api/projects/conversations')) {
        return jsonResponse([])
      }
      if (url.includes('/workspace/api/projects/metadata')) {
        return jsonResponse({ branch: 'main', commit: 'abcdef0' })
      }
      if (url.includes('/attractor/status')) {
        return jsonResponse({ status: 'idle' })
      }
      if (url.includes('/attractor/api/flows')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-hidden-runs')) {
        return jsonResponse({
          runs: [
            buildRunRecord({
              flowName: 'review-hidden.dot',
              projectPath: '/tmp/project-hidden-runs',
              runId: 'run-hidden',
            }),
          ],
        })
      }
      if (url.includes('/attractor/pipelines/run-hidden/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-hidden',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'review',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-hidden/context')) {
        return jsonResponse({
          pipeline_id: 'run-hidden',
          context: { summary: 'alpha' },
        })
      }
      if (url.includes('/attractor/pipelines/run-hidden/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-hidden',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-hidden/graph-preview')) {
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
      if (url.includes('/attractor/pipelines/run-hidden/questions')) {
        return jsonResponse({
          pipeline_id: 'run-hidden',
          questions: [],
        })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.getState().registerProject('/tmp/project-hidden-runs')
      useStore.getState().setActiveProjectPath('/tmp/project-hidden-runs')
      useStore.getState().setViewMode('runs')
    })

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('review-hidden.dot')).toBeVisible()
    })

    const selectedRunRow = within(screen.getByTestId('run-list-scroll-region'))
      .getByText('review-hidden.dot')
      .closest('[data-testid="run-history-row"]')
    expect(selectedRunRow).not.toBeNull()
    await user.click(selectedRunRow!)

    const runEventSource = await waitFor(() => {
      const source = findEventSource('/attractor/pipelines/run-hidden/events')
      expect(source).not.toBeNull()
      return source
    })

    await user.click(screen.getByTestId('nav-mode-projects'))

    act(() => {
      runEventSource?.emitMessage(stableTimelineEvent(1, {
        type: 'StageStarted',
        node_id: 'review',
        index: 2,
      }))
    })

    await user.click(screen.getByTestId('nav-mode-runs'))

    await waitFor(() => {
      expect(
        within(screen.getByTestId('run-event-timeline-panel')).getByText('Stage review started'),
      ).toBeVisible()
    })
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
          return new Response(JSON.stringify({ status: 'idle' }), {
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
          return new Response(JSON.stringify({ status: 'idle' }), {
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
    expect(screen.getByTestId('canvas-workspace-primary')).toHaveAttribute('data-canvas-active', 'true')
    expect(screen.getByTestId('execution-workspace-primary')).toHaveAttribute('data-execution-active', 'false')

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
    expect(screen.getByTestId('execution-launch-input-context.request.summary')).toHaveValue('Review the auth flow')

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
