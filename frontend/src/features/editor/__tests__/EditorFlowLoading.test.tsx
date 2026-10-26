import { Editor } from '@/features/editor/Editor'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { EditorPreviewResponse } from '@/features/editor/services/editorPreview'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import {
  loadEditorFlowPayload,
  loadEditorPreview,
} from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

vi.mock('@/features/editor/components/ValidationPanel', () => ({
  ValidationPanel: () => null,
}))

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

const createDeferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const buildPreview = (nodeCount: number): EditorPreviewResponse => ({
  status: 'ok',
  graph: {
    nodes: Array.from({ length: nodeCount }, (_, index) => ({
      id: `node_${index + 1}`,
      label: `Node ${index + 1}`,
      shape: 'box',
      prompt: `Prompt ${index + 1}`,
    })),
    edges: [],
    graph_attrs: {},
  },
  diagnostics: [],
  errors: [],
})

const buildManagerPreview = (options?: {
  expanded?: boolean
  managerLabel?: string
}): EditorPreviewResponse => ({
  status: 'ok',
  graph: {
    nodes: [
      { id: 'start', label: 'Start', shape: 'Mdiamond' },
      {
        id: 'manager',
        label: options?.managerLabel ?? 'Manager',
        shape: 'house',
        type: 'stack.manager_loop',
      },
    ],
    edges: [
      { from: 'start', to: 'manager' },
    ],
    graph_attrs: {},
    ...(options?.expanded
      ? {
          child_previews: {
            manager: {
              flow_name: 'child-worker.dot',
              flow_path: '/tmp/child-worker.dot',
              flow_label: 'Child Worker',
              read_only: true,
              provenance: 'derived_child_preview',
              graph: {
                graph_attrs: {},
                nodes: [
                  { id: 'child_start', label: 'Child Start', shape: 'Mdiamond' },
                ],
                edges: [],
              },
            },
          },
        }
      : {}),
  },
  diagnostics: [],
  errors: [],
})

const resetEditorState = (activeFlow: string | null) => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'editor',
    preferredAdvancedControls: false,
    preferredExpandChildFlows: false,
    preferredGraphSettingsOpen: false,
    preferredEditorMode: 'structured',
    clientPreferencesLoaded: false,
    activeFlow,
    executionFlow: null,
    suppressPreview: true,
    graphAttrs: {},
    editorExpandChildFlowsByFlow: {},
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    selectedNodeId: null,
    selectedEdgeId: null,
    uiDefaults: {
      llm_provider: 'openai',
      llm_model: 'gpt-5.4-mini',
      reasoning_effort: 'high',
    },
  }))
}

const renderEditor = () => render(
  <ReactFlowProvider>
    <Editor />
  </ReactFlowProvider>,
)

describe('Editor flow loading behavior', () => {
  beforeEach(() => {
    resetEditorState('flow-a.dot')
    localStorage.setItem('spark.debug.performance', '1')
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({
      name: flowName,
      files: [{ name: 'flow.md', content: `---\nid: ${flowName}\n---\n` }],
      flow: { id: flowName },
    }))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('opens the preferred files mode after hydration without persisting flow resets', async () => {
    vi.mocked(loadEditorPreview).mockResolvedValue(buildPreview(1))
    useStore.setState({ preferredEditorMode: 'files', clientPreferencesLoaded: true })
    const persist = vi.fn()
    window.addEventListener('spark:preferences-completed', persist)
    try {
      renderEditor()
      await waitFor(() => expect(useStore.getState().editorMode).toBe('files'))
      expect(await screen.findByTestId('flow-file-editor')).toHaveValue('---\nid: flow-a.dot\n---\n')
      expect(persist).not.toHaveBeenCalled()
    } finally { window.removeEventListener('spark:preferences-completed', persist) }
  })

  it('previews the loaded flow as JSON', async () => {
    vi.mocked(loadEditorPreview).mockResolvedValue(buildPreview(1))
    renderEditor()
    await waitFor(() => expect(loadEditorPreview).toHaveBeenCalled())
    expect(vi.mocked(loadEditorPreview).mock.calls[0][0]).toEqual({ id: 'flow-a.dot' })
  })

  it('clears the previous canvas when the next flow load fails', async () => {
    const previewDeferreds = new Map<string, Deferred<EditorPreviewResponse>>([
      ['flow-a.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-b.dot', createDeferred<EditorPreviewResponse>()],
    ])
    vi.mocked(loadEditorPreview).mockImplementation((flow) => {
      const deferred = previewDeferreds.get(String(flow?.id))
      if (!deferred) {
        throw new Error(`Unexpected preview request: ${flow?.id}`)
      }
      return deferred.promise
    })

    renderEditor()
    const profile = await screen.findByTestId('canvas-performance-profile')

    act(() => {
      previewDeferreds.get('flow-a.dot')?.resolve(buildPreview(1))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '1'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))

    act(() => {
      previewDeferreds.get('flow-b.dot')?.reject(new Error('preview failed'))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))
  })

  it('ignores a superseded flow load that resolves after a newer selection', async () => {
    const previewDeferreds = new Map<string, Deferred<EditorPreviewResponse>>([
      ['flow-a.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-b.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-c.dot', createDeferred<EditorPreviewResponse>()],
    ])
    vi.mocked(loadEditorPreview).mockImplementation((flow) => {
      const deferred = previewDeferreds.get(String(flow?.id))
      if (!deferred) {
        throw new Error(`Unexpected preview request: ${flow?.id}`)
      }
      return deferred.promise
    })

    renderEditor()
    const profile = await screen.findByTestId('canvas-performance-profile')

    act(() => {
      previewDeferreds.get('flow-a.dot')?.resolve(buildPreview(1))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '1'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-c.dot' }))
    })

    act(() => {
      previewDeferreds.get('flow-c.dot')?.resolve(buildPreview(3))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '3'))

    act(() => {
      previewDeferreds.get('flow-b.dot')?.resolve(buildPreview(2))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '3'))
  })

  it('reuses child-flow expansion preferences while preserving explicit per-flow choices and read-only previews', async () => {
    const user = userEvent.setup()
    vi.mocked(loadEditorPreview).mockImplementation(async (_flow, _init, options) => {
      const flowName = options?.flowName ?? 'unknown.dot'
      if (flowName === 'flow-a.dot') {
        return buildManagerPreview({ expanded: options?.expandChildren === true })
      }
      if (flowName === 'flow-b.dot') {
        return buildManagerPreview({ managerLabel: 'Secondary Manager' })
      }
      throw new Error(`Unexpected preview request for ${flowName}`)
    })

    renderEditor()

    await screen.findByTestId('editor-child-flow-toggle')
    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren !== true),
      ).toBe(true)
    })

    expect(screen.getByRole('button', { name: '+ Node' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Expanded' }))

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Read-only preview. Switch to Parent only to edit.')
    expect(screen.getAllByText('Read-only Preview').length).toBeGreaterThan(0)
    expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-a.dot']).toBe(true)

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-b.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Read-only preview. Switch to Parent only to edit.')
    await user.click(screen.getByRole('button', { name: 'Parent' }))
    await waitFor(() => {
      expect(
        screen.queryByText('Read-only preview. Switch to Parent only to edit.'),
      ).not.toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: '+ Node' })).toBeInTheDocument()
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-a.dot']).toBe(true)
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-b.dot']).toBe(false)

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-a.dot' }))
    })

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Read-only preview. Switch to Parent only to edit.')
    expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
  })
})

describe('Editor file view', () => {
  const files = [
    { name: 'flow.md', content: '---\nid: demo\n---\nThe flow.\n' },
    { name: 'node_1.md', content: '---\nkind: agent_task\n---\nPrompt 1\n' },
  ]

  beforeEach(() => {
    resetEditorState('demo')
    resetFlowSaveBaselines()
    localStorage.setItem('spark.debug.performance', '1')
    useStore.setState({
      editorMode: 'structured',
      preferredEditorMode: 'files',
      clientPreferencesLoaded: true,
      activeProjectPath: '/tmp/project',
    })
    vi.mocked(loadEditorFlowPayload).mockReset()
    vi.mocked(loadEditorFlowPayload).mockResolvedValue({ name: 'demo', layer: 'org', files, flow: { id: 'demo' } })
    vi.mocked(loadEditorPreview).mockReset()
    vi.mocked(loadEditorPreview).mockResolvedValue(buildPreview(1))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('lists the flow files and selects the file of the node selected on the canvas', async () => {
    const user = userEvent.setup()
    renderEditor()
    const editor = await screen.findByTestId('flow-file-editor')
    expect(screen.getAllByTestId('flow-file').map((row) => row.getAttribute('data-file'))).toEqual(['flow.md', 'node_1.md'])
    expect(editor).toHaveValue(files[0].content)

    act(() => useStore.setState({ selectedNodeId: 'node_1' }))
    await waitFor(() => expect(screen.getByTestId('flow-file-editor')).toHaveValue(files[1].content))

    await user.click(screen.getByRole('button', { name: 'flow.md' }))
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(files[0].content)
  })

  it('saves the edited file as files, then reloads the flow onto the canvas', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ status: 'saved', name: 'demo' }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    renderEditor()
    const editor = await screen.findByTestId('flow-file-editor')
    const edited = '---\nid: demo\n---\nThe edited flow.\n'
    fireEvent.change(editor, { target: { value: edited } })
    vi.mocked(loadEditorFlowPayload).mockResolvedValue({
      name: 'demo', layer: 'org', files: [{ name: 'flow.md', content: edited }, files[1]], flow: { id: 'demo' },
    })
    vi.mocked(loadEditorPreview).mockResolvedValue(buildPreview(2))

    await user.click(screen.getByTestId('flow-files-save'))

    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(url).toContain('/api/flows')
    expect(JSON.parse(String(init.body))).toEqual({
      name: 'demo',
      project_path: '/tmp/project',
      files: [{ name: 'flow.md', content: edited }],
    })
    await waitFor(() => expect(loadEditorFlowPayload).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.getByTestId('canvas-performance-profile')).toHaveAttribute('data-node-count', '2'))
    expect(useStore.getState().editorMode).toBe('files')
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(edited)
    expect(screen.getByTestId('flow-files-save')).toBeDisabled()
  })

  it('shows a validation error against the file and does not reload', async () => {
    const user = userEvent.setup()
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({
      detail: { status: 'validation_error', error: 'flow.md: missing id', diagnostics: [], errors: [] },
    }), { status: 422 })))
    renderEditor()
    const editor = await screen.findByTestId('flow-file-editor')
    fireEvent.change(editor, { target: { value: '---\n---\n' } })

    await user.click(screen.getByTestId('flow-files-save'))

    const error = await screen.findByTestId('flow-files-error')
    expect(error).toHaveTextContent('flow.md')
    expect(error).toHaveTextContent('missing id')
    expect(loadEditorFlowPayload).toHaveBeenCalledTimes(1)
    expect(screen.getByTestId('flow-file-editor')).toHaveValue('---\n---\n')
  })
})
