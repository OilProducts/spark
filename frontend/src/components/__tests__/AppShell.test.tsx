import App from '@/App'
import { useStore } from '@/store'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetAppShellState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    selectedRunId: null,
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

const installCanvasWorkspaceFetchMock = () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      if (url.includes('/attractor/api/flows/simple-linear.dot')) {
        return new Response(JSON.stringify({
          name: 'simple-linear.dot',
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
      if (url.includes('/attractor/api/flows/implement-review-loop.dot')) {
        return new Response(JSON.stringify({
          name: 'implement-review-loop.dot',
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
        return new Response(JSON.stringify(['simple-linear.dot', 'implement-review-loop.dot']), {
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
    window.HTMLElement.prototype.scrollIntoView = vi.fn()
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
    expect(screen.getByTestId('top-nav-active-project')).toHaveTextContent('No active project')
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

    expect(screen.getByTestId('top-nav-active-project')).toHaveTextContent('/tmp/project-shell')
  })

  it('keeps the shell mounted when selecting a flow in editor mode', async () => {
    const user = userEvent.setup()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/attractor/api/flows/simple-linear.dot')) {
          return new Response(JSON.stringify({
            name: 'simple-linear.dot',
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
          return new Response(JSON.stringify(['simple-linear.dot']), {
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
        if (url.includes('/workspace/api/flows/simple-linear.dot?surface=human')) {
          return new Response(JSON.stringify({
            name: 'simple-linear.dot',
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
    await user.click(await screen.findByRole('button', { name: /simple-linear\.dot/i }))

    await waitFor(() => {
      expect(screen.getByTestId('app-shell')).toBeVisible()
      expect(screen.getByTestId('top-nav')).toBeVisible()
      expect(screen.getByTestId('inspector-panel')).toBeVisible()
    })
  })

  it('keeps editor and execution sessions isolated across tab switches', async () => {
    const user = userEvent.setup()
    installCanvasWorkspaceFetchMock()

    render(<App />)

    await user.click(screen.getByTestId('nav-mode-execution'))
    await user.click(await screen.findByRole('button', { name: /implement-review-loop\.dot/i }))
    expect(await screen.findByTestId('execution-launch-inputs')).toBeVisible()

    await user.type(
      screen.getByTestId('execution-launch-input-context.request.summary'),
      'Review the auth flow',
    )

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(screen.getByTestId('inspector-panel')).toBeVisible()
    expect(screen.getByTestId('editor-no-flow-state')).toHaveTextContent('Select a flow to begin authoring.')
    expect(screen.getByTestId('editor-workspace')).toBeVisible()
    expect(screen.getByTestId('execution-workspace').className).toContain('hidden')

    await user.click(await screen.findByRole('button', { name: /simple-linear\.dot/i }))
    await waitFor(() => {
      expect(screen.getByTestId('editor-mode-toggle')).toBeVisible()
    })
    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const rawDotEditor = await screen.findByTestId('raw-dot-editor')
    await user.type(rawDotEditor, '\n// editor draft note')

    await user.click(screen.getByTestId('nav-mode-execution'))
    expect(await screen.findByTestId('execution-launch-input-context.request.summary')).toHaveValue('Review the auth flow')

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect((await screen.findByTestId('raw-dot-editor') as HTMLTextAreaElement).value).toContain('// editor draft note')
  })

  it('anchors execution launch inputs to the canvas panel instead of the sidebar shell', async () => {
    const user = userEvent.setup()
    installCanvasWorkspaceFetchMock()

    render(<App />)

    await user.click(screen.getByTestId('nav-mode-execution'))
    await user.click(await screen.findByRole('button', { name: /implement-review-loop\.dot/i }))

    const executionCanvasPanel = await screen.findByTestId('execution-canvas-panel')
    const executionFooterControls = screen.getByTestId('execution-footer-controls')

    expect(executionCanvasPanel).toHaveClass('relative')
    expect(executionCanvasPanel.parentElement).toHaveClass('min-w-0')
    expect(executionCanvasPanel).toContainElement(executionFooterControls)
  })
})
