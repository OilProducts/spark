import { GraphSettings } from '@/features/editor/GraphSettings'
import { SettingsPanel } from '@/features/settings/SettingsPanel'
import { StylesheetEditor } from '@/features/editor/components/StylesheetEditor'
import { generateFlowYaml } from '@/lib/flowYamlUtils'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const TEST_GRAPH_FLOW = 'test-graph.yaml'

const resetGraphSettingsState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'editor',
    activeProjectPath: '/tmp/project-graph-settings',
    activeFlow: TEST_GRAPH_FLOW,
    executionFlow: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
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
    },
  }))
}

const wrapWithFlowProvider = (node: ReactNode) => render(<ReactFlowProvider>{node}</ReactFlowProvider>)

describe('Graph and settings behavior', () => {
  beforeEach(() => {
    resetGraphSettingsState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
        const method = init?.method ?? 'GET'
        if (url.includes('/workspace/api/flows/') && url.includes('/launch-policy') && method === 'PUT') {
          const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}
          return new Response(
            JSON.stringify({
              name: TEST_GRAPH_FLOW,
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

  it('persists global LLM defaults from settings panel inputs', async () => {
    const user = userEvent.setup()
    render(<SettingsPanel />)

    expect(document.querySelector('#settings-llm-model-options option[value="gpt-5.4"]')).toBeTruthy()
    await user.clear(screen.getByPlaceholderText('openai'))
    await user.type(screen.getByPlaceholderText('openai'), 'anthropic')
    await user.clear(screen.getByLabelText('Default LLM Model'))
    await user.type(screen.getByLabelText('Default LLM Model'), 'claude-3.7-sonnet')
    const reasoningSelect = screen
      .getAllByRole('combobox')
      .find((element) => element.tagName === 'SELECT')
    expect(reasoningSelect).toBeDefined()
    await user.selectOptions(reasoningSelect as HTMLSelectElement, 'xhigh')

    expect(useStore.getState().uiDefaults).toEqual({
      llm_provider: 'anthropic',
      llm_model: 'claude-3.7-sonnet',
      llm_profile: '',
      reasoning_effort: 'xhigh',
    })
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
    expect(screen.getByTestId('flow-metadata-help')).toHaveTextContent('FlowDefinition defaults')
    expect(screen.getByRole('button', { name: 'Apply To Nodes' })).toBeEnabled()
    const graphReasoningSelect = screen.getByLabelText('Default Reasoning Effort') as HTMLSelectElement
    expect(graphReasoningSelect.querySelector('option[value="xhigh"]')).toBeTruthy()

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

    await waitFor(() => {
      expect(screen.getByTestId('graph-launch-policy-status')).toHaveTextContent(
        'Workspace flow catalog settings saved.',
      )
    })

    expect(screen.getByLabelText('Lock Scope')).toHaveValue('project')
    expect(screen.getByLabelText('Lock Key')).toHaveValue('main-worktree-integration')
    expect(screen.getByLabelText('Conflict Policy')).toHaveValue('queue')
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
