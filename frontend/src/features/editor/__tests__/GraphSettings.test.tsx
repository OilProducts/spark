import { chooseModel, customModel, chooseEffort, openPicker } from '@/components/model-chooser/__tests__/picker'
import { GraphSettings } from '@/features/editor/GraphSettings'
import { SettingsPanel } from '@/features/settings/SettingsPanel'
import { StylesheetEditor } from '@/features/editor/components/StylesheetEditor'
import { generateFlowYaml } from '@/lib/flowYamlUtils'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const TEST_GRAPH_FLOW = 'test-graph.yaml'
let serverModels = { provider: 'openai' as string | null, llm_profile: null as string | null, model: 'gpt-5.3' as string | null, reasoning_effort: 'high' as string | null }
let settingsRevision = 1

const resetGraphSettingsState = () => {
  {
useStore.setState({...useStore.getState(),
viewMode: 'editor',
activeProjectPath: '/tmp/project-graph-settings',
activeFlow: TEST_GRAPH_FLOW,
executionFlow: null,
workingDir: DEFAULT_WORKING_DIRECTORY,
projectRegistry: {
      '/tmp/project-graph-settings': {
        directoryPath: '/tmp/project-graph-settings',
        isFavorite: false,
        lastAccessedAt: null,
      },
    },
projectSessionsByPath: {
      '/tmp/project-graph-settings': {
        workingDir: DEFAULT_WORKING_DIRECTORY,
        conversationId: null,




      },
    },
projectRegistrationError: null,
recentProjectPaths: ['/tmp/project-graph-settings'],
flowMetadata: {},
flowMetadataErrors: {},
flowMetadataUserEditVersion: 0,
graphAttrs: {},
graphAttrErrors: {},
graphAttrsUserEditVersion: 0,
preferredAdvancedControls: false,
preferredExpandChildFlows: false,
preferredGraphSettingsOpen: false,
editorGraphSettingsPanelOpenByFlow: {},
editorShowAdvancedFlowMetadataByFlow: {},
editorShowAdvancedGraphAttrsByFlow: {},
editorLaunchInputDraftsByFlow: {},
editorLaunchInputDraftErrorByFlow: {},
editorNodeInspectorSessionsByNodeId: {},
saveState: 'idle',
saveStateVersion: 0,
saveErrorMessage: null,
saveErrorKind: null,
diagnostics: [],
nodeDiagnostics: {},
edgeDiagnostics: {},
hasValidationErrors: false,
uiDefaults: {
      llm_provider: 'openai',
      llm_model: 'gpt-5.3',
      reasoning_effort: 'high',
    }});
useStore.getState().setRunsSelectedRunId(null);
}
}

const wrapWithFlowProvider = (node: ReactNode) => render(<ReactFlowProvider>{node}</ReactFlowProvider>)
const modelDefaultsCard = () => within(screen.getByRole('heading', { name: 'Model defaults (Workspace)' }).closest<HTMLElement>('[data-slot=card]')!)

describe('Graph and settings behavior', () => {
  beforeEach(() => {
    resetGraphSettingsState()
    serverModels = { provider: 'openai', llm_profile: null, model: 'gpt-5.3', reasoning_effort: 'high' }
    settingsRevision = 1
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
        const method = init?.method ?? 'GET'
        if (url.includes('/chat-models')) return Response.json({ models: [{ provider: 'anthropic', id: 'claude-sonnet-4-6', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh', 'max'] }, { provider: 'codex', id: 'gpt-5.5' }, { provider: 'openai', id: 'gpt-5.3', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh'] }], providers: { codex: { status: 'available', error: null } }, provider_reasoning_efforts: { openai: ['low', 'medium', 'high', 'xhigh'], anthropic: ['low', 'medium', 'high', 'xhigh', 'max'] } })
        if (url.endsWith('/workspace/api/settings')) {
          if (method === 'PATCH') {
            const body = JSON.parse(String(init?.body))
            if (body.expected_revision !== String(settingsRevision)) return Response.json({ detail: 'Settings changed.' }, { status: 409 })
            serverModels = body.value
            settingsRevision += 1
          }
          return Response.json({ models: { scope: 'workspace', source: 'workspace', revision: String(settingsRevision), stored: serverModels, effective: serverModels } })
        }
        if (url.includes('/workspace/api/flows/') && url.includes('/launch-policy') && method === 'PUT') {
          const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}
          return new Response(
            JSON.stringify({
              name: TEST_GRAPH_FLOW,
              revision: 'catalog-revision',
              launch_policy: body.launch_policy ?? 'agent_requestable',
              effective_launch_policy: body.launch_policy ?? 'agent_requestable',
              execution_lock: body.execution_lock ?? null,
              allowed_launch_policies: ['agent_requestable', 'trigger_only', 'disabled'],
              allowed_execution_lock_scopes: ['project'],
              allowed_execution_lock_conflict_policies: ['queue'],
            }),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        if (url.includes('/workspace/api/flows/')) {
          return new Response(
            JSON.stringify({
              name: TEST_GRAPH_FLOW,
              title: 'Implement From Plan File',
              description: 'Snapshot a plan file, implement it, and iterate until complete.',
              revision: 'catalog-revision',
              launch_policy: null,
              effective_launch_policy: 'disabled',
              execution_lock: null,
            }),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({ status: 'saved' }), {
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

  it('keeps atomic model and effort edits local until explicit Save', async () => {
    const user = userEvent.setup()
    useStore.setState({ activeProjectPath: null })
    render(<SettingsPanel />)
    await waitFor(() => expect(screen.getByRole('button', { name: /^Model:/ })).toBeEnabled())
    await chooseModel(user, 'anthropic', 'claude-sonnet-4-6')
    expect(serverModels.provider).toBe('openai')
    await chooseEffort(user, 'XHigh')
    expect(useStore.getState().uiDefaults.llm_provider).toBe('openai')
    await user.click(modelDefaultsCard().getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved. Applies to the next message.')
    expect(serverModels).toEqual({ provider: 'anthropic', llm_profile: null, model: 'claude-sonnet-4-6', reasoning_effort: 'xhigh', thinking: null, thinking_budget_tokens: null, reasoning_mode: null, reasoning_summary: null })
    await chooseModel(user, 'Codex', 'gpt-5.5')
    await user.click(modelDefaultsCard().getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent('claude-sonnet-4-6'))
  })

  it('keeps custom model drafts through failed saves and reloads persisted values on Discard', async () => {
    const user = userEvent.setup()
    useStore.setState({ activeProjectPath: null })
    serverModels = { provider: 'openai', llm_profile: null, model: 'private-model', reasoning_effort: 'high' }
    render(<SettingsPanel />)
    await screen.findByRole('button', { name: /Model: private-model/ })
    await customModel(user, 'custom:next')
    settingsRevision += 1
    await user.click(modelDefaultsCard().getByRole('button', { name: /^Save/ }))
    await screen.findByText(/responded with HTTP 409/)
    expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent('custom:next')
    expect(serverModels.model).toBe('private-model')
    await user.click(modelDefaultsCard().getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent('private-model'))
  })

  it('uses configured profile models and saves mutually exclusive provider/profile selectors', async () => {
    const user = userEvent.setup()
    const originalFetch = fetch
    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).includes('/llm-profiles')) return Promise.resolve(Response.json({ profiles: [{
        id: 'team', provider: 'openai_compatible', models: ['team-model'], default_model: 'team-model', configured: true,
      }] }))
      return originalFetch(input, init)
    }))
    render(<SettingsPanel />)
    await chooseModel(user, 'openai_compatible / team', 'team-model')
    await user.click(modelDefaultsCard().getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(serverModels).toMatchObject({ provider: null, llm_profile: 'team', model: 'team-model' }))
  })

  it('loads provider-dependent discovery without changing saved defaults', async () => {
    const user = userEvent.setup()
    let resolve!: (response: Response) => void
    const originalFetch = fetch
    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => String(input).includes('/chat-models')
      ? new Promise<Response>((done) => { resolve = done }) : originalFetch(input, init)))
    const saved = useStore.getState().uiDefaults
    render(<SettingsPanel />)
    await openPicker(user)
    expect(screen.getByText('Loading models…')).toHaveAttribute('role', 'status')
    await act(async () => resolve(Response.json({ models: [
      { provider: 'openai', id: 'discovered-openai', display: 'OpenAI' },
      { provider: 'anthropic', id: 'discovered-anthropic', display: 'Anthropic' },
    ], providers: { codex: { status: 'available', error: null } } })))
    expect(screen.queryByText('Loading models…')).toBeNull()
    expect(useStore.getState().uiDefaults).toEqual(saved)
    expect(await screen.findByRole('option', { name: 'OpenAI' })).toBeVisible()
    await user.click(screen.getByRole('option', { name: 'Anthropic' }))
    expect(serverModels.model).toBe('gpt-5.3')
  })

  it.each(['rejected', 'unavailable'] as const)('shows %s discovery feedback without overwriting defaults', async (failure) => {
    const originalFetch = fetch
    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      if (!String(input).includes('/chat-models')) return originalFetch(input, init)
      return failure === 'rejected' ? Promise.reject(new Error('offline')) : Promise.resolve(Response.json({
        models: [], providers: { codex: { status: 'unavailable', error: 'CLI unavailable' } },
      }))
    }))
    if (failure === 'unavailable') serverModels.provider = 'codex'
    const saved = useStore.getState().uiDefaults
    render(<SettingsPanel />)
    await openPicker(userEvent.setup())
    await screen.findAllByText(/Model discovery unavailable\./)
    expect(useStore.getState().uiDefaults).toEqual(saved)
    expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent(saved.llm_model)
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
  })

  it.each([false, true])('ignores stale discovery responses (rejected: %s)', async (rejectOld) => {
    const requests: { resolve: (response: Response) => void; reject: (error: Error) => void }[] = []
    const originalFetch = fetch
    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => String(input).includes('/chat-models')
      ? new Promise<Response>((resolve, reject) => { requests.push({ resolve, reject }) }) : originalFetch(input, init)))
    render(<SettingsPanel />)
    await waitFor(() => expect(requests).toHaveLength(1))
    act(() => { window.dispatchEvent(new Event('spark:settings-live-event')) })
    expect(requests).toHaveLength(2)
    const payload = (id: string) => Response.json({ models: [{ provider: 'openai', id, display: id }], providers: { codex: { status: 'available', error: null } } })
    await act(async () => requests[1].resolve(payload('current-model')))
    await act(async () => {
      if (rejectOld) requests[0].reject(new Error('old failure'))
      else requests[0].resolve(payload('stale-model'))
    })
    await openPicker(userEvent.setup())
    expect(await screen.findByRole('option', { name: 'current-model' })).toBeVisible()
    expect(screen.queryByRole('option', { name: 'stale-model' })).toBeNull()
    expect(screen.queryByText('Loading models…')).toBeNull()
    expect(useStore.getState().uiDefaults.llm_model).toBe('gpt-5.3')
  })

  it('highlights stylesheet tokens and emits changes through textarea editing', async () => {
    const onChange = vi.fn()
    const initialStylesheet = '#review {\n  llm_model: "gpt-5";\n}'

    const { container } = render(<StylesheetEditor value={initialStylesheet} onChange={onChange} />)

    const highlight = screen.getByTestId('model-stylesheet-editor-highlight')
    expect(highlight).toBeVisible()
    expect(container.querySelector('[data-token-type="selector"]')).toBeTruthy()
    expect(container.querySelector('[data-token-type="property"]')).toBeTruthy()

    const textarea = screen.getByRole('textbox')
    fireEvent.change(textarea, { target: { value: '* llm_provider_openai' } })

    expect(onChange).toHaveBeenCalledWith('* llm_provider_openai')
  })

  it('validates typed FlowDefinition defaults and exposes only extension metadata in advanced settings', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.getByTestId('graph-structured-form')).toBeVisible()
    expect(screen.getByTestId('flow-metadata-help')).toHaveTextContent('Leave blank to omit the field from YAML output.')
    expect(screen.getByRole('button', { name: 'Apply To Nodes' })).toBeEnabled()
    await openPicker(user)
    expect(screen.getByRole('button', { name: 'XHigh' })).toBeVisible()
    await user.keyboard('{Escape}')

    const fidelityInput = screen.getByPlaceholderText('full')
    await user.clear(fidelityInput)
    await user.type(fidelityInput, 'invalid')

    expect(screen.getByText(/Fidelity default must be one of/i)).toBeVisible()
    expect(useStore.getState().flowMetadataErrors.fidelity).toContain('Fidelity default must be one of')

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.queryByTestId('graph-model-stylesheet-editor')).not.toBeInTheDocument()
    expect(screen.queryByTestId('graph-scoped-defaults-section')).not.toBeInTheDocument()
    expect(screen.queryByTestId('graph-subgraphs-section')).not.toBeInTheDocument()
  })

  it.each([
    { graphThinking: 'off', workspaceThinking: 'budget', graphBudget: '', expectedThinking: 'off', expectedBudget: null },
    { graphThinking: 'adaptive', workspaceThinking: 'budget', graphBudget: '', expectedThinking: 'adaptive', expectedBudget: null },
    { graphThinking: '', workspaceThinking: '', graphBudget: '', expectedThinking: '', expectedBudget: null },
    { graphThinking: '', workspaceThinking: 'budget', graphBudget: '', expectedThinking: 'budget', expectedBudget: 2048 },
    { graphThinking: 'budget', workspaceThinking: 'budget', graphBudget: '', expectedThinking: 'budget', expectedBudget: 2048 },
    { graphThinking: 'budget', workspaceThinking: 'budget', graphBudget: '4096', expectedThinking: 'budget', expectedBudget: 4096 },
  ])('Apply To Nodes resolves thinking and budget: $graphThinking / $workspaceThinking / $graphBudget', async ({
    graphThinking, workspaceThinking, graphBudget, expectedThinking, expectedBudget,
  }) => {
    const user = userEvent.setup()
    useStore.setState({
      flowMetadata: { thinking: graphThinking, thinking_budget_tokens: graphBudget },
      uiDefaults: { ...useStore.getState().uiDefaults, thinking: workspaceThinking, thinking_budget_tokens: workspaceThinking ? '2048' : '' },
    })
    render(
      <ReactFlowProvider initialNodes={[{
        id: 'review', position: { x: 0, y: 0 },
        data: { label: 'Review', thinking: 'budget', thinking_budget_tokens: '8192' },
      }]}>
        <GraphSettings inline />
      </ReactFlowProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Apply To Nodes' }))

    await waitFor(() => {
      const request = vi.mocked(fetch).mock.calls.find(([, init]) => init?.method === 'POST')
      expect(request).toBeDefined()
      const { content } = JSON.parse(String(request![1]?.body))
      const nodeYaml = content.slice(content.indexOf('\nnodes:'))
      expect(nodeYaml).toContain('\nnodes:\n  review:')
      if (expectedThinking) expect(nodeYaml).toContain(`thinking: ${expectedThinking}`)
      else expect(nodeYaml).not.toContain('thinking:')
      if (expectedBudget === null) expect(nodeYaml).not.toContain('thinking_budget_tokens:')
      else expect(nodeYaml).toContain(`thinking_budget_tokens: ${expectedBudget}`)
    })
  })

  it('surfaces FlowDefinition title and description fields without leaking them into extension attrs', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setFlowMetadata({
        title: '  Implement From Plan File  ',
        description: '  Snapshot a plan file, implement it, and iterate until complete.  ',
        custom_attr: 'keep me',
      } as Record<string, string>)
    })

    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.getByLabelText('Title')).toHaveValue('Implement From Plan File')
    expect(screen.getByLabelText('Description')).toHaveValue(
      'Snapshot a plan file, implement it, and iterate until complete.',
    )

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attrs-list')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attr-key-0')).toHaveValue('custom_attr')
    expect(screen.queryByText('title')).not.toBeInTheDocument()
    expect(screen.queryByText('description')).not.toBeInTheDocument()

    const yaml = generateFlowYaml(TEST_GRAPH_FLOW, [], [], useStore.getState().flowMetadata)
    expect(yaml).toContain('title: "Implement From Plan File"')
    expect(yaml).toContain('description: "Snapshot a plan file, implement it, and iterate until complete."')
  })

  it('persists launch input declarations as FlowDefinition inputs', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.getByTestId('graph-launch-inputs-editor')).toBeVisible()
    await user.click(screen.getByTestId('graph-launch-input-add'))

    await user.type(screen.getByTestId('graph-launch-input-label-0'), 'Acceptance Criteria')
    await user.selectOptions(screen.getByTestId('graph-launch-input-type-0'), 'string[]')
    await user.clear(screen.getByTestId('graph-launch-input-key-0'))
    await user.type(screen.getByTestId('graph-launch-input-key-0'), 'context.request.acceptance_criteria')
    await user.type(
      screen.getByTestId('graph-launch-input-description-0'),
      'One acceptance criterion per line in the execution form.',
    )
    await user.click(screen.getByTestId('graph-launch-input-required-0'))

    expect(screen.queryByTestId('graph-launch-inputs-error')).not.toBeInTheDocument()
    expect(useStore.getState().flowMetadata.inputs).toBe(
      JSON.stringify([
        {
          key: 'context.request.acceptance_criteria',
          label: 'Acceptance Criteria',
          type: 'string[]',
          description: 'One acceptance criterion per line in the execution form.',
          required: true,
        },
      ]),
    )

    const yaml = generateFlowYaml(TEST_GRAPH_FLOW, [], [], useStore.getState().flowMetadata)
    expect(yaml).toContain('inputs:')
    expect(yaml).toContain('key: context.request.acceptance_criteria')
  })

  it('authors FlowDefinition defaults without DOT defaults or subgraph payloads', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.queryByTestId('graph-scoped-defaults-section')).not.toBeInTheDocument()
    expect(screen.queryByTestId('graph-subgraph-add')).not.toBeInTheDocument()

    await user.clear(screen.getByLabelText('Default Max Retries'))
    await user.type(screen.getByLabelText('Default Max Retries'), '3')
    await user.clear(screen.getByPlaceholderText('full'))
    await user.type(screen.getByPlaceholderText('full'), 'summary:high')

    const state = useStore.getState()
    const yaml = generateFlowYaml(TEST_GRAPH_FLOW, [], [], state.flowMetadata)
    expect(yaml).toContain('schema_version: "1.0"')
    expect(yaml).toContain('defaults:')
    expect(yaml).toContain('max_retries: 3')
    expect(yaml).toContain('fidelity: summary:high')
    expect(yaml).not.toContain('legacy default prompt')
    expect(yaml).not.toContain('cluster_legacy')
  })

  it('does not derive FlowDefinition node kind or YAML metadata from DOT shape/type attrs', () => {
    const yaml = generateFlowYaml(
      TEST_GRAPH_FLOW,
      [
        {
          id: 'review',
          position: { x: 0, y: 0 },
          data: {
            label: 'Review',
            shape: 'hexagon',
            type: 'wait.human',
          },
        },
      ],
      [],
      {},
    )

    expect(yaml).toContain('kind: agent_task')
    expect(yaml).not.toContain('human_gate')
    expect(yaml).not.toContain('wait.human')
    expect(yaml).not.toContain('shape:')
    expect(yaml).not.toContain('type:')
  })

  it('preserves panel state, advanced toggle, and invalid launch-input drafts across remounts', async () => {
    const user = userEvent.setup()
    const firstRender = wrapWithFlowProvider(<GraphSettings />)

    await user.click(screen.getByRole('button', { name: 'Graph Settings' }))
    await waitFor(() => {
      expect(screen.getByTestId('graph-structured-form')).toBeVisible()
    })
    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toBeVisible()
    })

    await user.click(screen.getByTestId('graph-launch-input-add'))
    fireEvent.change(screen.getByTestId('graph-launch-input-label-0'), {
      target: { value: 'Broken Draft' },
    })
    fireEvent.change(screen.getByTestId('graph-launch-input-key-0'), {
      target: { value: 'draft.invalid' },
    })
    await user.click(screen.getByTestId('graph-advanced-toggle'))

    expect(screen.getByTestId('graph-launch-inputs-error')).toHaveTextContent(
      'Context keys must use the context.* namespace: draft.invalid',
    )
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()

    firstRender.unmount()
    wrapWithFlowProvider(<GraphSettings />)

    expect(screen.getByTestId('graph-structured-form')).toBeVisible()
    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toBeVisible()
    })
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.getByTestId('graph-launch-input-key-0')).toHaveValue('draft.invalid')
    expect(screen.getByTestId('graph-launch-inputs-error')).toHaveTextContent(
      'Context keys must use the context.* namespace: draft.invalid',
    )
  })

  it('loads and saves workspace launch policy without touching flow save state', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'No catalog entry yet. Effective policy is Disabled.',
      )
    })
    expect(screen.getByLabelText('Launch Policy')).toHaveValue('disabled')

    await user.selectOptions(screen.getByLabelText('Launch Policy'), 'agent_requestable')
    expect(vi.mocked(fetch).mock.calls.filter(([, init]) => init?.method === 'PUT')).toHaveLength(0)
    await user.click(screen.getByRole('button', { name: /^Save/ }))

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'Workspace flow catalog settings saved.',
      )
    })

    expect(useStore.getState().saveState).toBe('idle')
    expect(screen.getByLabelText('Launch Policy')).toHaveValue('agent_requestable')
  })

  it('loads and saves execution lock config from the workspace flow catalog', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'No catalog entry yet. Effective policy is Disabled.',
      )
    })

    await user.click(screen.getByLabelText('Enable execution lock'))
    await user.type(screen.getByLabelText('Lock Key'), 'main-worktree-integration')
    fireEvent.blur(screen.getByLabelText('Lock Key'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'Workspace flow catalog settings saved.',
      )
    })

    expect(screen.getByLabelText('Lock Scope')).toHaveValue('project')
    expect(screen.getByLabelText('Lock Key')).toHaveValue('main-worktree-integration')
    expect(screen.getByLabelText('Conflict Policy')).toHaveValue('queue')
  })

  it('retains a stale policy draft and reloads the catalog on Discard', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)
    await waitFor(() => expect(screen.getByRole('button', { name: /^Discard/ })).toBeEnabled())
    await user.selectOptions(screen.getByLabelText('Launch Policy'), 'agent_requestable')
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(Response.json({ detail: 'Settings changed; reload before saving.' }, { status: 409 }))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent('Settings changed'))
    expect(screen.getByLabelText('Launch Policy')).toHaveValue('agent_requestable')
    const request = fetchMock.mock.calls.find(([, init]) => init?.method === 'PUT')
    expect(JSON.parse(String(request?.[1]?.body)).expected_revision).toBe('catalog-revision')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeEnabled()
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(screen.getByLabelText('Launch Policy')).toHaveValue('disabled'))
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
  })

  it('does not autosave when graph attrs are replaced from hydrated state', async () => {
    const fetchMock = vi.mocked(fetch)
    wrapWithFlowProvider(<GraphSettings inline />)

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'No catalog entry yet. Effective policy is Disabled.',
      )
    })

    const saveRequestsBefore = fetchMock.mock.calls.filter(([, init]) => (init?.method ?? 'GET') === 'POST').length

    act(() => {
      useStore.getState().replaceFlowMetadata({
        goal: 'Hydrated goal',
      })
    })

    await new Promise((resolve) => window.setTimeout(resolve, 300))

    const saveRequestsAfter = fetchMock.mock.calls.filter(([, init]) => (init?.method ?? 'GET') === 'POST').length
    expect(saveRequestsAfter).toBe(saveRequestsBefore)
    expect(useStore.getState().flowMetadataUserEditVersion).toBe(0)
    expect(useStore.getState().saveState).toBe('idle')
  })
})
