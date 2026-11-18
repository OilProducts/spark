import { chooseModel, customModel, chooseEffort, openPicker } from '@/components/model-chooser/__tests__/picker'
import { GraphSettings } from '@/features/editor/GraphSettings'
import { SettingsPanel } from '@/features/settings/SettingsPanel'
import {
  buildCanonicalFlowModelFromEditorState,
  buildCanonicalFlowModelFromPreviewGraph,
  buildFlowDefinitionFromCanonicalFlowModel,
  type CanonicalFlowDefinition,
} from '@/lib/canonicalFlowModel'
import { useStore } from '@/store'
import { ReactFlowProvider, useReactFlow } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const TEST_GRAPH_FLOW = 'test-graph'
const buildFlow = (
  nodes: Parameters<typeof buildCanonicalFlowModelFromEditorState>[1]['nodes'],
  edges: Parameters<typeof buildCanonicalFlowModelFromEditorState>[1]['edges'],
  flowMetadata: Parameters<typeof buildCanonicalFlowModelFromEditorState>[1]['flowMetadata'],
  flow: CanonicalFlowDefinition | null = null,
) => buildFlowDefinitionFromCanonicalFlowModel(
  TEST_GRAPH_FLOW,
  buildCanonicalFlowModelFromEditorState(TEST_GRAPH_FLOW, { nodes, edges, flowMetadata, flow }),
)
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
triggersSession: useStore.getInitialState().triggersSession,
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
const modelDefaultsCard = () => within(screen.getByRole('heading', { name: 'Model defaults (workspace)' }).closest<HTMLElement>('[data-slot=card]')!)

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
        if (url.includes('/chat-models')) return Response.json({ models: [{ provider: 'anthropic', id: 'claude-sonnet-4-6', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh', 'max'] }, { provider: 'codex', id: 'gpt-5.5' }, { provider: 'openai', id: 'gpt-5.3', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh'] }], providers: { codex: { status: 'available', error: null, label: 'Codex' } }, provider_reasoning_efforts: { openai: ['low', 'medium', 'high', 'xhigh'], anthropic: ['low', 'medium', 'high', 'xhigh', 'max'] } })
        if (url.endsWith('/workspace/api/settings')) {
          if (method === 'PATCH') {
            const body = JSON.parse(String(init?.body))
            if (body.expected_revision !== String(settingsRevision)) return Response.json({ detail: 'Settings changed.' }, { status: 409 })
            serverModels = body.value
            settingsRevision += 1
          }
          return Response.json({ models: { scope: 'workspace', source: 'workspace', revision: String(settingsRevision), stored: serverModels, effective: serverModels } })
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
    await screen.findByText(/Settings changed\./)
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

  it('validates typed FlowDefinition defaults and exposes only extension metadata in advanced settings', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings />)

    expect(screen.getByTestId('graph-structured-form')).toBeVisible()
    expect(screen.getByTestId('flow-metadata-help')).toHaveTextContent('Leave blank to omit the field from the flow.')
    // With no agent step, there is nothing to apply the defaults to, and it says so.
    expect(screen.getByRole('button', { name: 'Apply to agent steps' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Apply to agent steps' })).toHaveAccessibleDescription('This flow has no agent steps, the only steps that run a model.')
    await openPicker(user)
    expect(screen.getByRole('button', { name: 'XHigh' })).toBeVisible()
    await user.keyboard('{Escape}')

    await user.clear(screen.getByLabelText('Default max retries'))
    await user.type(screen.getByLabelText('Default max retries'), '-1')

    expect(screen.getByText('Max retries default must be a non-negative integer.')).toBeVisible()
    expect(useStore.getState().flowMetadataErrors.max_retries).toContain('non-negative integer')

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
  ])('Apply to agent steps resolves thinking and budget: $graphThinking / $workspaceThinking / $graphBudget', async ({
    graphThinking, workspaceThinking, graphBudget, expectedThinking, expectedBudget,
  }) => {
    const user = userEvent.setup()
    useStore.setState({
      flowMetadata: { thinking: graphThinking, thinking_budget_tokens: graphBudget },
      uiDefaults: { ...useStore.getState().uiDefaults, thinking: workspaceThinking, thinking_budget_tokens: workspaceThinking ? '2048' : '' },
    })
    render(
      <ReactFlowProvider defaultNodes={[{
        id: 'review', position: { x: 0, y: 0 },
        data: { label: 'Review', kind: 'agent_task', thinking: 'budget', thinking_budget_tokens: '8192' },
      }, { id: 'check', position: { x: 0, y: 50 }, data: { label: 'Check', kind: 'tool', 'tool.command': 'true' } },
      { id: 'done', position: { x: 0, y: 100 }, data: { label: 'Done', kind: 'exit' } }]} defaultEdges={[
        { id: 'review-check', source: 'review', target: 'check' }, { id: 'check-done', source: 'check', target: 'done' },
      ]}>
        <GraphSettings />
      </ReactFlowProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Apply to 1 agent step' }))

    await waitFor(() => {
      const request = vi.mocked(fetch).mock.calls.find(([, init]) => init?.method === 'POST')
      expect(request).toBeDefined()
      const { flow } = JSON.parse(String(request![1]?.body))
      expect(Object.keys(flow.nodes)).toEqual(['review', 'check', 'done'])
      // Only agent steps run a model.
      expect(JSON.stringify(flow.nodes.check)).not.toMatch(/llm_|reasoning|thinking/)
      const review = JSON.stringify(flow.nodes.review)
      if (expectedThinking) expect(review).toContain(`"thinking":"${expectedThinking}"`)
      else expect(review).not.toContain('"thinking"')
      if (expectedBudget === null) expect(review).not.toContain('"thinking_budget_tokens"')
      else expect(review).toContain(`"thinking_budget_tokens":${expectedBudget}`)
    })
    expect(screen.getByTestId('graph-apply-defaults-status')).toHaveTextContent('Set 1 agent step to these defaults.')
  })

  it('Apply to agent steps says so when its agent steps already use the defaults, until a step changes', async () => {
    const user = userEvent.setup()
    useStore.setState({ flowMetadata: { thinking: 'off' } })
    // As an Undo or the step's own inspector would.
    function EditReview() {
      const { setNodes } = useReactFlow()
      return <button type="button" onClick={() => setNodes((nodes) => nodes.map((node) => node.id === 'review' ? { ...node, data: { ...node.data, thinking: 'adaptive' } } : node))}>Edit review</button>
    }
    render(
      <ReactFlowProvider defaultNodes={[
        { id: 'review', position: { x: 0, y: 0 }, data: { label: 'Review', kind: 'agent_task', llm_model: 'gpt-5.3', llm_provider: 'openai', reasoning_effort: 'high', thinking: 'off' } },
        { id: 'done', position: { x: 0, y: 100 }, data: { label: 'Done', kind: 'exit' } },
      ]} defaultEdges={[{ id: 'review-done', source: 'review', target: 'done' }]}>
        <GraphSettings />
        <EditReview />
      </ReactFlowProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Apply to 1 agent step' }))
    expect(screen.getByTestId('graph-apply-defaults-status')).toHaveTextContent('Its agent step already uses these defaults.')
    expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === 'POST')).toBe(false)
    await user.click(screen.getByRole('button', { name: 'Edit review' }))
    expect(screen.getByTestId('graph-apply-defaults-status')).toBeEmptyDOMElement()
  })

  it('surfaces FlowDefinition title and description fields without leaking them into extension attrs', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setFlowMetadata({ custom_attr: 'keep me' } as Record<string, string>)
    })

    wrapWithFlowProvider(<GraphSettings />)

    // Typed key by key: spaces must survive between keystrokes.
    await user.type(screen.getByLabelText('Title'), ' Implement from plan file ')
    await user.type(screen.getByLabelText('Description'), 'Snapshot a plan file, implement it, and iterate until complete. ')
    expect(screen.getByLabelText('Title')).toHaveValue(' Implement from plan file ')

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attrs-list')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attr-key-0')).toHaveValue('custom_attr')
    expect(screen.queryByText('title')).not.toBeInTheDocument()
    expect(screen.queryByText('description')).not.toBeInTheDocument()

    const flow = buildFlow([], [], useStore.getState().flowMetadata)
    expect(flow.title).toBe('Implement from plan file')
    expect(flow.description).toBe('Snapshot a plan file, implement it, and iterate until complete.')

    // Clearing the description clears it from the flow, not back to what it was.
    await user.clear(screen.getByLabelText('Description'))
    expect(buildFlow([], [], useStore.getState().flowMetadata, flow).description).toBe('')
  })

  it('names the triggers that start this flow without a required launch input, and opens one', async () => {
    const user = userEvent.setup()
    const trigger = (id: string, static_context: Record<string, unknown>) => ({
      id, project_path: '/tmp/project-graph-settings', name: `Trigger ${id}`, enabled: true, protected: false, revision: 'r1', source_type: 'schedule' as const,
      created_at: '', updated_at: '', action: { flow_name: TEST_GRAPH_FLOW, static_context }, source: {}, state: { recent_history: [] },
    })
    useStore.setState((state) => ({
      triggersSession: { ...state.triggersSession, triggers: [trigger('nightly', {}), trigger('weekly', { 'context.request.version': '2.0.0' })] },
      flowMetadata: { inputs: JSON.stringify([{ key: 'context.request.version', label: 'Release version', type: 'string', description: '', required: true }]) },
    }))
    wrapWithFlowProvider(<GraphSettings />)

    const gaps = await screen.findByTestId('graph-launch-inputs-trigger-gaps')
    expect(gaps).toHaveTextContent('A trigger doesn’t set a required input, so a run it starts fails:Trigger nightly needs Release version')
    expect(gaps).not.toHaveTextContent('Trigger weekly')
    await user.click(within(gaps).getByRole('button', { name: 'Trigger nightly' }))
    expect(useStore.getState().viewMode).toBe('triggers')
  })

  it('persists launch input declarations as FlowDefinition inputs', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings />)

    expect(screen.getByTestId('graph-launch-inputs-editor')).toBeVisible()
    await user.click(screen.getByTestId('graph-launch-input-add'))
    // A new input starts in the flow's own namespace, never a flow-specific one.
    expect(screen.getByTestId('graph-launch-input-key-0')).toHaveValue('context.')

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

    const flow = buildFlow([], [], useStore.getState().flowMetadata)
    expect(flow.inputs).toEqual([expect.objectContaining({ key: 'context.request.acceptance_criteria' })])
  })

  it('authors FlowDefinition defaults without DOT defaults or subgraph payloads', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings />)

    expect(screen.queryByTestId('graph-scoped-defaults-section')).not.toBeInTheDocument()
    expect(screen.queryByTestId('graph-subgraph-add')).not.toBeInTheDocument()

    await user.clear(screen.getByLabelText('Default max retries'))
    await user.type(screen.getByLabelText('Default max retries'), '3')

    const state = useStore.getState()
    const flow = buildFlow([], [], state.flowMetadata)
    expect(flow.schema_version).toBe('2')
    expect(flow.defaults).toMatchObject({ max_retries: 3 })
    expect(JSON.stringify(flow)).not.toContain('legacy default prompt')
    expect(JSON.stringify(flow)).not.toContain('cluster_legacy')
  })

  it('round-trips a node session through runtime config', () => {
    const flow = buildFlow(
      [{ id: 'task', position: { x: 0, y: 0 }, data: { label: 'Task', kind: 'agent_task', session: 'work' } }],
      [],
      {},
    )
    expect((flow.nodes as Record<string, { runtime?: Record<string, unknown> }>).task.runtime).toEqual({ session: 'work' })

    const model = buildCanonicalFlowModelFromPreviewGraph(TEST_GRAPH_FLOW, { nodes: [{ id: 'task' }], edges: [] }, { flow })
    expect(model.nodes.find((node) => node.id === 'task')?.attrs.session).toBe('work')
  })

  it('does not derive FlowDefinition node kind or metadata from DOT shape/type attrs', () => {
    const flow = buildFlow(
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

    const review = JSON.stringify((flow.nodes as Record<string, unknown>).review)
    expect(review).toContain('"kind":"agent_task"')
    expect(review).not.toContain('human_gate')
    expect(review).not.toContain('wait.human')
    expect(review).not.toContain('"shape"')
    expect(review).not.toContain('"type"')
  })

  it('preserves the advanced toggle and invalid launch-input drafts across remounts', async () => {
    const user = userEvent.setup()
    const firstRender = wrapWithFlowProvider(<GraphSettings />)

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
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.getByTestId('graph-launch-input-key-0')).toHaveValue('draft.invalid')
    expect(screen.getByTestId('graph-launch-inputs-error')).toHaveTextContent(
      'Context keys must use the context.* namespace: draft.invalid',
    )
  })

  it('edits the execution lock key as FlowDefinition execution_lock', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings />)

    await user.type(screen.getByLabelText('One at a time (lock name)'), 'integration')
    expect(useStore.getState().flowMetadata.execution_lock).toBe('integration')
    expect(buildFlow([], [], useStore.getState().flowMetadata).execution_lock)
      .toEqual({ scope: 'project', key: 'integration', conflict_policy: 'queue' })

    await user.clear(screen.getByLabelText('One at a time (lock name)'))
    const base = { execution_lock: { scope: 'project', key: 'integration', conflict_policy: 'queue' } }
    expect(buildFlow([], [], useStore.getState().flowMetadata, base).execution_lock).toBeUndefined()
    expect(buildFlow([], [], {}, base).execution_lock).toEqual(base.execution_lock)
    expect(buildCanonicalFlowModelFromPreviewGraph('g', { nodes: [], edges: [] }, { flow: base }).flowMetadata.execution_lock)
      .toBe('integration')

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.queryByTestId('graph-extension-attr-key-0')).not.toBeInTheDocument()
  })

  it('does not autosave when graph attrs are replaced from hydrated state', async () => {
    const fetchMock = vi.mocked(fetch)
    wrapWithFlowProvider(<GraphSettings />)

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
