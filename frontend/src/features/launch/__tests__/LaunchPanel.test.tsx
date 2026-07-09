import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { DialogProvider } from '@/components/app/dialog-controller'
import { useStore } from '@/store'
import { LaunchPanel, type LaunchPanelProps } from '@/features/launch'

const TEST_FLOW = 'test-linear.yaml'
const TEST_FLOW_CONTENT = 'schema_version: "1"\nid: simple_linear\n'
const TEST_PROJECT = '/tmp/project'

const jsonResponse = (payload: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  })

const buildPreviewPayload = (inputs: Array<Record<string, unknown>> = []) => ({
  status: 'ok',
  flow: { inputs },
  graph: {
    metadata: {},
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

const installLaunchFetchMock = (options?: {
  inputs?: Array<Record<string, unknown>>
  projectMetadataStatus?: number
  startStatus?: 'started' | 'queued'
  pipelineId?: string
}) => {
  const inputs = options?.inputs ?? []
  const pipelineId = options?.pipelineId ?? 'run-123'
  const startStatus = options?.startStatus ?? 'started'
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    if (url.includes('/workspace/api/projects/metadata')) {
      if (options?.projectMetadataStatus && options.projectMetadataStatus >= 400) {
        return jsonResponse({ detail: 'metadata unavailable' }, { status: options.projectMetadataStatus })
      }
      return jsonResponse({ branch: 'main' })
    }
    if (url.endsWith('/workspace/api/settings')) {
      return jsonResponse({
        execution_placement: {
          execution_modes: ['native'],
          config: {
            filename: 'execution-profiles.toml',
            path: '/tmp/config/execution-profiles.toml',
            exists: false,
            loaded: true,
            synthesized_native_default: true,
          },
          default_execution_profile_id: null,
          profiles: [
            { id: 'native', label: 'Native', mode: 'native', enabled: true, image: null, capabilities: {}, metadata: {} },
          ],
          validation_errors: [],
        },
      })
    }
    if (url.includes('/workspace/api/flows/')) {
      return jsonResponse({
        name: TEST_FLOW,
        title: TEST_FLOW,
        description: '',
        launch_policy: null,
        effective_launch_policy: 'disabled',
        execution_lock: null,
      })
    }
    if (url.includes(`/attractor/api/flows/`)) {
      return jsonResponse({ name: TEST_FLOW, content: TEST_FLOW_CONTENT })
    }
    if (url.endsWith('/attractor/preview') && init?.method === 'POST') {
      return jsonResponse(buildPreviewPayload(inputs))
    }
    if (url.endsWith('/attractor/pipelines') && init?.method === 'POST') {
      return jsonResponse({ status: startStatus, pipeline_id: pipelineId }, { status: 202 })
    }
    return jsonResponse({})
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

const renderLaunchPanel = (overrides?: Partial<LaunchPanelProps>) => {
  const onLaunched = vi.fn()
  const onClose = vi.fn()
  const props: LaunchPanelProps = {
    target: {
      flowName: TEST_FLOW,
      loadFlowContent: async () => TEST_FLOW_CONTENT,
      previewSource: { kind: 'flow', flowName: TEST_FLOW },
    },
    projectPath: TEST_PROJECT,
    initialWorkingDirectory: TEST_PROJECT,
    onLaunched,
    onClose,
    ...overrides,
  }
  render(
    <DialogProvider>
      <LaunchPanel {...props} />
    </DialogProvider>,
  )
  return { onLaunched, onClose }
}

const startedPipelinePayload = (fetchMock: ReturnType<typeof vi.fn>) => {
  const startCall = fetchMock.mock.calls.find(([input, init]) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : (input as Request).url
    return url.endsWith('/attractor/pipelines') && (init as RequestInit | undefined)?.method === 'POST'
  })
  if (!startCall) {
    return null
  }
  return JSON.parse(String((startCall[1] as RequestInit).body))
}

describe('LaunchPanel', () => {
  beforeEach(() => {
    useStore.setState((state) => ({
      ...state,
      activeProjectPath: TEST_PROJECT,
      projectRegistry: {},
      runtimeStatus: 'idle',
      runtimeOutcome: null,
      selectedRunId: null,
    }))
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('launches the flow with typed launch inputs and reports the run id', async () => {
    const fetchMock = installLaunchFetchMock({
      inputs: [{ key: 'context.topic', label: 'Topic', type: 'string', required: true }],
    })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    const topicInput = await screen.findByTestId('execution-launch-input-context.topic')
    await user.type(topicInput, 'quarterly report')

    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)

    await waitFor(() => expect(onLaunched).toHaveBeenCalledWith('run-123', false))
    const payload = startedPipelinePayload(fetchMock)
    expect(payload).toMatchObject({
      flow_content: TEST_FLOW_CONTENT,
      flow_name: TEST_FLOW,
      working_directory: TEST_PROJECT,
      launch_context: { 'context.topic': 'quarterly report' },
    })
    expect(useStore.getState().selectedRunId).toBe('run-123')
    expect(useStore.getState().runtimeStatus).toBe('running')
  })

  it('blocks launching when a required input is missing', async () => {
    installLaunchFetchMock({
      inputs: [{ key: 'context.topic', label: 'Topic', type: 'string', required: true }],
    })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    await screen.findByTestId('execution-launch-input-context.topic')
    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)

    await screen.findByTestId('run-start-error-banner')
    expect(onLaunched).not.toHaveBeenCalled()
  })

  it('disables launching without an active project', async () => {
    installLaunchFetchMock()
    renderLaunchPanel({ projectPath: null })

    const startButton = await screen.findByTestId('launch-panel-start-button')
    expect(startButton).toBeDisabled()
    expect(startButton).toHaveAttribute('title', 'Select an active project before running.')
  })

  it('asks for confirmation when the git policy gate cannot verify project state', async () => {
    const fetchMock = installLaunchFetchMock({ projectMetadataStatus: 500 })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)

    await screen.findByTestId('shared-dialog')
    await user.click(screen.getByTestId('shared-dialog-cancel'))

    await waitFor(() => {
      expect(screen.getByTestId('run-start-git-policy-warning-banner')).toBeInTheDocument()
    })
    expect(startedPipelinePayload(fetchMock)).toBeNull()
    expect(onLaunched).not.toHaveBeenCalled()
  })

  it('prefills launch inputs from initial values', async () => {
    installLaunchFetchMock({
      inputs: [{ key: 'context.topic', label: 'Topic', type: 'string', required: true }],
    })
    renderLaunchPanel({ initialInputValues: { 'context.topic': 'previous topic' } })

    const topicInput = await screen.findByTestId('execution-launch-input-context.topic')
    await waitFor(() => expect(topicInput).toHaveValue('previous topic'))
  })
})
