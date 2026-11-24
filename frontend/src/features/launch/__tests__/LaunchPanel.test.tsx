import { selectSelectedRunId, selectSelectedRunSession } from '@/state/runsSessionSelectors'
import { act, render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { DialogProvider } from '@/components/app/dialog-controller'
import { useStore } from '@/store'
import { LaunchPanel, useExecutionPlacement, type LaunchPanelProps } from '@/features/launch'
import { deferred, switchEnvironment } from '@/test/environments'

const TEST_FLOW = 'test-linear.yaml'
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
      { from: 'start', to: 'task', kind: 'route', decision: 'next', description: 'Continue.', label: '' },
      { from: 'task', to: 'done', kind: 'route', decision: 'next', description: 'Continue.', label: '' },
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
        execution_lock: null,
      })
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
    {
useStore.setState({...useStore.getState(),
activeProjectPath: TEST_PROJECT,
projectRegistry: {}});
useStore.getState().setRunsSelectedRunId(null);
}
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
    // The editor flushes its pending save before the launch reads the flow by name.
    const prepareLaunch = vi.fn(async () => {
      expect(startedPipelinePayload(fetchMock)).toBeNull()
    })
    const { onLaunched } = renderLaunchPanel({
      target: { flowName: TEST_FLOW, prepareLaunch, previewSource: { kind: 'flow', flowName: TEST_FLOW } },
    })

    const topicInput = await screen.findByTestId('execution-launch-input-context.topic')
    await user.type(topicInput, 'quarterly report')
    await user.type(screen.getByLabelText(/Run name/), '  Q3 report  ')

    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)

    await waitFor(() => expect(onLaunched).toHaveBeenCalledWith('run-123', false))
    expect(prepareLaunch).toHaveBeenCalledOnce()
    // The launch form previews the saved flow by name.
    const previewCall = fetchMock.mock.calls.find(([input]) => String(input).endsWith('/attractor/preview'))
    expect(JSON.parse(String((previewCall?.[1] as RequestInit).body))).toEqual({ flow_name: TEST_FLOW, project_path: TEST_PROJECT })
    expect(fetchMock.mock.calls.some(([input]) => String(input).includes('/attractor/api/flows/'))).toBe(false)
    const payload = startedPipelinePayload(fetchMock)
    expect(payload).toMatchObject({
      flow_name: TEST_FLOW,
      working_directory: TEST_PROJECT,
      launch_context: { 'context.topic': 'quarterly report' },
      title: 'Q3 report',
    })
    expect(selectSelectedRunId(useStore.getState())).toBe('run-123')
    expect(selectSelectedRunSession(useStore.getState())?.record).toBeNull()
  })

  it('blocks launching when a required input is missing, saying so under the field and focusing it', async () => {
    installLaunchFetchMock({
      inputs: [{ key: 'context.topic', label: 'Topic', type: 'string', required: true }],
    })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    const input = await screen.findByTestId('execution-launch-input-context.topic')
    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)

    const error = await screen.findByTestId('execution-launch-input-error-context.topic')
    expect(error).toHaveTextContent('Topic is required.')
    expect(input).toHaveAttribute('aria-invalid', 'true')
    expect(input.getAttribute('aria-describedby')).toContain(error.id)
    await waitFor(() => expect(input).toHaveFocus())
    expect(screen.queryByTestId('run-start-error-banner')).toBeNull()
    expect(onLaunched).not.toHaveBeenCalled()

    // Typing clears the field's error.
    await user.type(input, 'Billing')
    expect(screen.queryByTestId('execution-launch-input-error-context.topic')).toBeNull()
  })

  it('hands focus to the first required input once loaded, and keeps Enter for new lines in a multi-line one, running on Ctrl+Enter', async () => {
    const fetchMock = installLaunchFetchMock({
      inputs: [
        { key: 'context.notes', label: 'Notes', type: 'string', multiline: true },
        { key: 'context.version', label: 'Version', type: 'string', required: true },
      ],
    })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    const version = await screen.findByTestId('execution-launch-input-context.version')
    await waitFor(() => expect(version).toHaveFocus())
    await waitFor(() => expect(screen.getByTestId('launch-panel-start-button')).toBeEnabled())
    expect(screen.getByTestId('launch-panel-run-shortcut')).toHaveTextContent('Enter runs from any field')
    await user.keyboard('1.4.0')
    await user.click(screen.getByTestId('execution-launch-input-context.notes'))
    await user.keyboard('Line one{Enter}Line two')
    expect(screen.getByTestId('execution-launch-input-context.notes')).toHaveValue('Line one\nLine two')
    expect(onLaunched).not.toHaveBeenCalled()
    await user.keyboard('{Control>}{Enter}{/Control}')

    await waitFor(() => expect(onLaunched).toHaveBeenCalledWith('run-123', false))
    expect(startedPipelinePayload(fetchMock)).toMatchObject({ launch_context: { 'context.version': '1.4.0', 'context.notes': 'Line one\nLine two' } })
  })

  it('runs on Enter from a one-line text input', async () => {
    const fetchMock = installLaunchFetchMock({
      inputs: [{ key: 'context.version', label: 'Version', type: 'string', required: true }],
    })
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()

    const version = await screen.findByTestId('execution-launch-input-context.version')
    await waitFor(() => expect(version).toHaveFocus())
    await waitFor(() => expect(screen.getByTestId('launch-panel-start-button')).toBeEnabled())
    expect(screen.queryByTestId('launch-panel-run-shortcut')).toBeNull()
    await user.keyboard('3.1.0{Enter}')

    await waitFor(() => expect(onLaunched).toHaveBeenCalledWith('run-123', false))
    expect(startedPipelinePayload(fetchMock)).toMatchObject({ launch_context: { 'context.version': '3.1.0' } })
  })

  it('closes on Escape from inside the panel', async () => {
    installLaunchFetchMock()
    const user = userEvent.setup()
    const { onClose } = renderLaunchPanel()

    expect(screen.getByTestId('launch-panel')).toHaveFocus()
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledTimes(1)

    await user.click(await screen.findByTestId('launch-panel-working-directory-input'))
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('disables launching without a project', async () => {
    installLaunchFetchMock()
    renderLaunchPanel({ projectPath: null })

    const startButton = await screen.findByTestId('launch-panel-start-button')
    expect(startButton).toBeDisabled()
    expect(startButton).toHaveAttribute('title', 'Choose a project before running.')
  })

  it('lists each execution profile once, says where it runs, and summarizes the chosen one', async () => {
    const launchFetch = installLaunchFetchMock()
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => String(input).endsWith('/workspace/api/settings')
      ? jsonResponse({ execution_placement: {
        execution_modes: ['native', 'local_container'],
        config: { filename: 'execution-profiles.toml', path: '/tmp/config/execution-profiles.toml', exists: true, loaded: true, synthesized_native_default: false },
        default_execution_profile_id: 'native',
        profiles: [
          { id: 'native', label: 'Native', mode: 'native', enabled: true, image: null, capabilities: {}, metadata: {} },
          { id: 'gpu', label: 'GPU box', mode: 'local_container', enabled: true, image: 'gpu:1', capabilities: {}, metadata: {} },
        ],
        validation_errors: [],
      } })
      : launchFetch(input, init)))
    renderLaunchPanel()

    const select = await screen.findByTestId<HTMLSelectElement>('execution-profile-override-select')
    await waitFor(() => expect(Array.from(select.options).map((option) => [option.value, option.textContent])).toEqual([
      ['', 'Native — on this computer (project default)'],
      ['gpu', 'GPU box — in a container'],
    ]))
    expect(screen.getByTestId('launch-panel-options-summary')).toHaveTextContent(/ · on this computer$/)

    const { result } = renderHook(() => useExecutionPlacement(true, TEST_PROJECT))
    await waitFor(() => expect(result.current.effectiveProfileSummary).toBe('on this computer'))
    act(() => result.current.setSelectedProfileId('gpu'))
    expect(result.current.effectiveProfileSummary).toBe('in a container: GPU box')
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

  it.each([
    ['the Git check', '/workspace/api/projects/metadata'],
    ['the launch request', '/attractor/pipelines'],
  ])('launches in the environment it started in, and follows it nowhere, after a switch during %s', async (_, heldPath) => {
    const fetchMock = installLaunchFetchMock()
    const answer = fetchMock.getMockImplementation()!
    const held = deferred<void>()
    const requested = () => fetchMock.mock.calls.map(([input]) => String(input))
    fetchMock.mockImplementation(async (input, init) => {
      if (String(input).includes(heldPath)) await held.promise
      return answer(input, init)
    })
    await switchEnvironment('env-r')
    const user = userEvent.setup()
    const { onLaunched } = renderLaunchPanel()
    const startButton = screen.getByTestId('launch-panel-start-button')
    await waitFor(() => expect(startButton).toBeEnabled())
    await user.click(startButton)
    await waitFor(() => expect(requested().some((url) => url.includes(heldPath))).toBe(true))

    await switchEnvironment(null)
    held.resolve()
    await waitFor(() => expect(startButton).toBeEnabled())
    expect(requested().filter((url) => url.endsWith('/attractor/pipelines')))
      .toEqual(['/workspace/api/environments/links/env-r/proxy/attractor/pipelines'])
    expect(onLaunched).not.toHaveBeenCalled()
    expect(selectSelectedRunId(useStore.getState())).toBeNull()
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
