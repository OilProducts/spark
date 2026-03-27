import { buildPipelineStartPayload } from '@/lib/pipelineStartPayload'
import { ExecutionControls } from '@/features/execution/ExecutionControls'
import { useStore } from '@/store'
import { DialogProvider } from '@/ui'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const TEST_LINEAR_FLOW = 'test-linear.dot'
const TEST_REVIEW_FLOW = 'test-review-loop.dot'
const TEST_SPEC_FLOW = 'test-spec-implementation.dot'

const jsonResponse = (payload: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  })

const renderExecutionControls = () =>
  render(
    <DialogProvider>
      <ExecutionControls />
    </DialogProvider>,
  )

const setViewportWidth = (width: number) => {
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: width,
  })
  window.dispatchEvent(new Event('resize'))
}

const buildPreviewPayload = (graphAttrs: Record<string, unknown> = {}) => ({
  status: 'ok',
  graph: {
    graph_attrs: graphAttrs,
    nodes: [
      { id: 'start', label: 'Start', shape: 'Mdiamond' },
      { id: 'task', label: 'Task', shape: 'box', prompt: 'Review request.' },
      { id: 'done', label: 'Done', shape: 'Msquare' },
    ],
    edges: [
      { from: 'start', to: 'task', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
      { from: 'task', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
    ],
  },
  diagnostics: [],
  errors: [],
})

const installExecutionFetchMock = (options?: {
  flowName?: string
  flowContent?: string
  graphAttrs?: Record<string, unknown>
  pipelineId?: string
}) => {
  const flowName = options?.flowName ?? TEST_LINEAR_FLOW
  const flowContent = options?.flowContent ?? 'digraph simple_linear { start -> done }'
  const graphAttrs = options?.graphAttrs ?? {}
  const pipelineId = options?.pipelineId ?? 'run-123'

  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    if (url.includes('/workspace/api/projects/metadata')) {
      return jsonResponse({ branch: 'main' })
    }
    if (url.endsWith(`/attractor/api/flows/${flowName}`)) {
      return jsonResponse({
        name: flowName,
        content: flowContent,
      })
    }
    if (url.endsWith('/attractor/preview') && init?.method === 'POST') {
      return jsonResponse(buildPreviewPayload(graphAttrs))
    }
    if (url.endsWith('/attractor/pipelines') && init?.method === 'POST') {
      return jsonResponse({ status: 'started', pipeline_id: pipelineId }, { status: 202 })
    }
    return jsonResponse({})
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

const resetExecutionState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    executionGraphAttrs: {},
    executionDiagnostics: [],
    executionNodeDiagnostics: {},
    executionEdgeDiagnostics: {},
    executionHasValidationErrors: false,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    runtimeStatus: 'idle',
    runtimeOutcome: null,
    runtimeOutcomeReasonCode: null,
    runtimeOutcomeReasonMessage: null,
    humanGate: null,
    projectRegistry: {},
    projectSessionsByPath: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    logs: [],
    nodeStatuses: {},
    selectedNodeId: null,
    selectedEdgeId: null,
  }))
}

describe('Execution controls behavior', () => {
  beforeEach(() => {
    setViewportWidth(1280)
    resetExecutionState()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('builds start payload with project, flow, backend model, and traceability ids', () => {
    const payload = buildPipelineStartPayload(
      {
        projectPath: '/tmp/project',
        flowSource: TEST_SPEC_FLOW,
        workingDirectory: '/tmp/project',
        backend: 'codex-app-server',
        model: 'gpt-5',
        specArtifactId: 'spec-123',
        planArtifactId: 'plan-456',
      },
      'digraph G { start -> done }',
    )

    expect(payload).toEqual({
      flow_content: 'digraph G { start -> done }',
      working_directory: '/tmp/project',
      backend: 'codex-app-server',
      model: 'gpt-5',
      flow_name: TEST_SPEC_FLOW,
      spec_id: 'spec-123',
      plan_id: 'plan-456',
    })
  })

  it('includes structured launch context in the start payload when provided', () => {
    const payload = buildPipelineStartPayload(
      {
        projectPath: '/tmp/project',
        flowSource: TEST_REVIEW_FLOW,
        workingDirectory: '/tmp/project',
        backend: 'codex-app-server',
        model: 'gpt-5',
        launchContext: {
          'context.request.summary': 'Add a health check endpoint.',
          'context.request.acceptance_criteria': ['GET /healthz returns 200'],
        },
        specArtifactId: null,
        planArtifactId: null,
      },
      'digraph G { start -> done }',
    )

    expect(payload).toEqual({
      flow_content: 'digraph G { start -> done }',
      working_directory: '/tmp/project',
      backend: 'codex-app-server',
      model: 'gpt-5',
      launch_context: {
        'context.request.summary': 'Add a health check endpoint.',
        'context.request.acceptance_criteria': ['GET /healthz returns 200'],
      },
      flow_name: TEST_REVIEW_FLOW,
      spec_id: null,
      plan_id: null,
    })
  })

  it('shows a launch-only empty state when no flow is selected', () => {
    useStore.setState((state) => ({
      ...state,
      viewMode: 'execution',
      activeProjectPath: '/tmp/project',
    }))

    renderExecutionControls()

    expect(screen.getByTestId('execution-launch-panel')).toBeVisible()
    expect(screen.getByTestId('execution-no-flow-state')).toHaveTextContent('Select a flow to launch.')
    expect(screen.queryByTestId('run-console-panel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-graph-panel')).not.toBeInTheDocument()
  })

  it('renders declared launch inputs and submits them as launch_context', async () => {
    const user = userEvent.setup()
    const fetchMock = installExecutionFetchMock({
      flowName: TEST_REVIEW_FLOW,
      flowContent: 'digraph implement_review_loop { start -> done }',
      graphAttrs: {
        'spark.launch_inputs': JSON.stringify([
          {
            key: 'context.request.summary',
            label: 'Request Summary',
            type: 'string',
            description: 'Brief description of the requested change.',
            required: true,
          },
          {
            key: 'context.request.target_paths',
            label: 'Target Paths',
            type: 'string[]',
            description: 'Optional files or directories to focus on.',
            required: false,
          },
          {
            key: 'context.request.acceptance_criteria',
            label: 'Acceptance Criteria',
            type: 'string[]',
            description: 'One acceptance criterion per line.',
            required: false,
          },
        ]),
      },
      pipelineId: 'run-555',
    })

    useStore.setState((state) => ({
      ...state,
      viewMode: 'execution',
      activeProjectPath: '/tmp/project',
      executionFlow: TEST_REVIEW_FLOW,
      projectSessionsByPath: {
        '/tmp/project': {
          workingDir: '/tmp/project',
          conversationId: null,
          projectEventLog: [],
          specId: null,
          specStatus: null,
          specProvenance: null,
          planId: 'plan-123',
          planStatus: 'draft',
          planProvenance: null,
        },
      },
    }))

    renderExecutionControls()

    expect(await screen.findByTestId('execution-launch-inputs')).toBeVisible()
    expect(screen.getByTestId('execution-launch-primary-action')).toContainElement(screen.getByTestId('execute-button'))
    expect(screen.getByTestId('execute-button')).toHaveTextContent('Run in project')
    expect(screen.getByRole('checkbox', { name: 'Open in Runs after launch' })).not.toBeChecked()

    await user.type(
      screen.getByTestId('execution-launch-input-context.request.summary'),
      'Add a health check endpoint',
    )
    await user.type(
      screen.getByTestId('execution-launch-input-context.request.target_paths'),
      'src/api/health.ts',
    )
    await user.type(
      screen.getByTestId('execution-launch-input-context.request.acceptance_criteria'),
      'GET /healthz returns 200{enter}Response body contains status ok',
    )

    await user.click(screen.getByTestId('execute-button'))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/attractor/pipelines',
        expect.objectContaining({
          method: 'POST',
        }),
      )
    })

    const pipelineCall = fetchMock.mock.calls.find(([input, init]) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      return url.endsWith('/attractor/pipelines') && init?.method === 'POST'
    })
    expect(pipelineCall).toBeDefined()
    const requestBody = JSON.parse(String(pipelineCall?.[1]?.body))
    expect(requestBody.launch_context).toEqual({
      'context.request.summary': 'Add a health check endpoint',
      'context.request.target_paths': [
        'src/api/health.ts',
      ],
      'context.request.acceptance_criteria': [
        'GET /healthz returns 200',
        'Response body contains status ok',
      ],
    })

    expect(useStore.getState().selectedRunId).toBe('run-555')
    expect(useStore.getState().viewMode).toBe('execution')
    expect(screen.getByTestId('execution-launch-success-notice')).toBeVisible()
  })

  it('stays on execution by default after launch and can jump to runs from the success notice', async () => {
    const user = userEvent.setup()
    installExecutionFetchMock({
      flowName: TEST_LINEAR_FLOW,
      pipelineId: 'run-stay',
    })

    useStore.setState((state) => ({
      ...state,
      viewMode: 'execution',
      activeProjectPath: '/tmp/project',
      executionFlow: TEST_LINEAR_FLOW,
      projectSessionsByPath: {
        '/tmp/project': {
          workingDir: '/tmp/project',
          conversationId: null,
          projectEventLog: [],
          specId: null,
          specStatus: null,
          specProvenance: null,
          planId: null,
          planStatus: 'draft',
          planProvenance: null,
        },
      },
    }))

    renderExecutionControls()

    await screen.findByTestId('execute-button')
    expect(screen.getByRole('checkbox', { name: 'Open in Runs after launch' })).not.toBeChecked()

    await user.click(screen.getByTestId('execute-button'))

    await waitFor(() => {
      expect(screen.getByTestId('execution-launch-success-notice')).toBeVisible()
    })
    expect(useStore.getState().viewMode).toBe('execution')

    await user.click(screen.getByTestId('execution-launch-success-view-run-button'))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(useStore.getState().selectedRunId).toBe('run-stay')
  })

  it('navigates to runs immediately when the post-launch checkbox is enabled', async () => {
    const user = userEvent.setup()
    installExecutionFetchMock({
      flowName: TEST_LINEAR_FLOW,
      pipelineId: 'run-open-in-runs',
    })

    useStore.setState((state) => ({
      ...state,
      viewMode: 'execution',
      activeProjectPath: '/tmp/project',
      executionFlow: TEST_LINEAR_FLOW,
      projectSessionsByPath: {
        '/tmp/project': {
          workingDir: '/tmp/project',
          conversationId: null,
          projectEventLog: [],
          specId: null,
          specStatus: null,
          specProvenance: null,
          planId: null,
          planStatus: 'draft',
          planProvenance: null,
        },
      },
    }))

    renderExecutionControls()

    await screen.findByTestId('execute-button')
    await user.click(screen.getByRole('checkbox', { name: 'Open in Runs after launch' }))
    expect(screen.getByRole('checkbox', { name: 'Open in Runs after launch' })).toBeChecked()

    await user.click(screen.getByTestId('execute-button'))

    await waitFor(() => {
      expect(useStore.getState().viewMode).toBe('runs')
    })
    expect(useStore.getState().selectedRunId).toBe('run-open-in-runs')
    expect(screen.queryByTestId('execution-launch-success-notice')).not.toBeInTheDocument()
  })
})
