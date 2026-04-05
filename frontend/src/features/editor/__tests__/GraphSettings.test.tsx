import { GraphSettings } from '@/features/editor/GraphSettings'
import { SettingsPanel } from '@/features/settings/SettingsPanel'
import { StylesheetEditor } from '@/features/editor/components/StylesheetEditor'
import { generateDot } from '@/lib/dotUtils'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'
const TEST_GRAPH_FLOW = 'test-graph.dot'

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
    runRecordOverrides: {},
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
        specId: null,
        specStatus: 'draft',
        planId: null,
        planStatus: 'draft',
      },
    },
    projectRegistrationError: null,
    recentProjectPaths: ['/tmp/project-graph-settings'],
    graphAttrs: {},
    graphAttrErrors: {},
    graphAttrsUserEditVersion: 0,
    editorGraphSettingsPanelOpenByFlow: {},
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
          return new Response(
            JSON.stringify({
              name: TEST_GRAPH_FLOW,
              launch_policy: 'agent_requestable',
              effective_launch_policy: 'agent_requestable',
              allowed_launch_policies: ['agent_requestable', 'trigger_only', 'disabled'],
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
              title: 'Implement Spec',
              description: 'Turn approved spec edits into execution plans.',
              launch_policy: null,
              effective_launch_policy: 'disabled',
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

    await user.clear(screen.getByPlaceholderText('openai'))
    await user.type(screen.getByPlaceholderText('openai'), 'anthropic')
    await user.clear(screen.getByPlaceholderText('gpt-5.2'))
    await user.type(screen.getByPlaceholderText('gpt-5.2'), 'claude-3.7-sonnet')
    const reasoningSelect = screen
      .getAllByRole('combobox')
      .find((element) => element.tagName === 'SELECT')
    expect(reasoningSelect).toBeDefined()
    await user.selectOptions(reasoningSelect as HTMLSelectElement, 'medium')

    expect(useStore.getState().uiDefaults).toEqual({
      llm_provider: 'anthropic',
      llm_model: 'claude-3.7-sonnet',
      reasoning_effort: 'medium',
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

  it('validates graph attrs and surfaces stylesheet diagnostics in advanced settings', async () => {
    const user = userEvent.setup()
    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.getByTestId('graph-structured-form')).toBeVisible()
    expect(screen.getByTestId('graph-attrs-help')).toHaveTextContent('Graph attributes are baseline defaults')
    expect(screen.getByRole('button', { name: 'Apply To Nodes' })).toBeEnabled()

    const fidelityInput = screen.getByPlaceholderText('full')
    await user.clear(fidelityInput)
    await user.type(fidelityInput, 'invalid')

    expect(screen.getByText(/Default fidelity must be one of/i)).toBeVisible()
    expect(useStore.getState().graphAttrErrors.default_fidelity).toContain('Default fidelity must be one of')

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-model-stylesheet-selector-guidance')).toBeVisible()
    expect(screen.getByTestId('graph-model-stylesheet-selector-preview')).toBeVisible()
    expect(screen.getByTestId('graph-model-stylesheet-effective-preview')).toBeVisible()

    const stylesheetInput = within(screen.getByTestId('graph-model-stylesheet-editor')).getByRole('textbox')
    await user.type(stylesheetInput, '#n1-model-stylesheet')
    expect(useStore.getState().graphAttrs.model_stylesheet).toContain('#n1')

    act(() => {
      useStore.getState().setDiagnostics([
        {
          rule_id: 'stylesheet_syntax',
          severity: 'error',
          message: 'Invalid stylesheet selector syntax.',
          line: 1,
        },
      ])
    })

    expect(screen.getByTestId('graph-model-stylesheet-diagnostics')).toHaveTextContent(
      'Invalid stylesheet selector syntax.',
    )
  })

  it('surfaces DOT-backed title and description fields without leaking them into extension attrs', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setGraphAttrs({
        'spark.title': '  Execution Planning  ',
        'spark.description': '  Turn approved spec edits into execution plans.  ',
        custom_attr: 'keep me',
      } as Record<string, string>)
    })

    wrapWithFlowProvider(<GraphSettings inline />)

    expect(screen.getByLabelText('Title')).toHaveValue('Execution Planning')
    expect(screen.getByLabelText('Description')).toHaveValue(
      'Turn approved spec edits into execution plans.',
    )

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-extension-attrs-editor')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attrs-list')).toBeVisible()
    expect(screen.getByTestId('graph-extension-attr-key-0')).toHaveValue('custom_attr')
    expect(screen.queryByText('spark.title')).not.toBeInTheDocument()
    expect(screen.queryByText('spark.description')).not.toBeInTheDocument()

    const dot = generateDot(TEST_GRAPH_FLOW, [], [], useStore.getState().graphAttrs)
    expect(dot).toContain('spark.title="Execution Planning"')
    expect(dot).toContain('spark.description="Turn approved spec edits into execution plans."')
  })

  it('persists launch input declarations as DOT-backed Spark metadata', async () => {
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
    expect(useStore.getState().graphAttrs['spark.launch_inputs']).toBe(
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

    const dot = generateDot(TEST_GRAPH_FLOW, [], [], useStore.getState().graphAttrs)
    expect(dot).toContain('spark.launch_inputs=')
    expect(dot).toContain('context.request.acceptance_criteria')
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
        'Workspace launch policy saved as Agent Requestable.',
      )
    })

    expect(useStore.getState().saveState).toBe('idle')
    expect(screen.getByLabelText('Launch Policy')).toHaveValue('agent_requestable')
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
      useStore.getState().replaceGraphAttrs({
        goal: 'Hydrated goal',
      })
    })

    await new Promise((resolve) => window.setTimeout(resolve, 300))

    const saveRequestsAfter = fetchMock.mock.calls.filter(([, init]) => (init?.method ?? 'GET') === 'POST').length
    expect(saveRequestsAfter).toBe(saveRequestsBefore)
    expect(useStore.getState().graphAttrsUserEditVersion).toBe(0)
    expect(useStore.getState().saveState).toBe('idle')
  })
})
