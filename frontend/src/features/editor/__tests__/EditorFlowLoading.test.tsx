import { Editor } from '@/features/editor/Editor'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { EditorPreviewResponse } from '@/features/editor/services/editorPreview'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import { ApiHttpError } from '@/lib/attractorClient'
import {
  loadEditorFlowFiles,
  loadEditorFlowPayload,
  loadEditorPreview,
} from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowFiles: vi.fn(),
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
    flowLoadFailure: null,
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

  it('says a linked flow its project doesn\'t have isn\'t there, with nothing to run', async () => {
    vi.mocked(loadEditorFlowPayload).mockRejectedValue(new ApiHttpError('/attractor/api/flows/nope', 404, 'Flow not found'))
    renderEditor()
    expect(await screen.findByTestId('editor-flow-load-failed')).toHaveTextContent('This flow isn’t here')
    expect(screen.getByTestId('editor-flow-load-failed')).toHaveTextContent('no flow named “flow-a.dot”')
    expect(screen.queryByRole('button', { name: /^Run/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(useStore.getState().activeFlow).toBeNull()
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
    await screen.findByText('Read-only preview. Switch to Parent to edit.')
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
    await screen.findByText('Read-only preview. Switch to Parent to edit.')
    await user.click(screen.getByRole('button', { name: 'Parent' }))
    await waitFor(() => {
      expect(
        screen.queryByText('Read-only preview. Switch to Parent to edit.'),
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
    await screen.findByText('Read-only preview. Switch to Parent to edit.')
    expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
  })
})

describe('Editor structured metadata', () => {
  const metadata = {
    software_development: { launcher: true },
    arbitrary_settings: { stages: [{ name: 'one', retries: [1, 2] }, null], owner: null, enabled: false },
  }
  const flow = {
    schema_version: '2',
    id: 'bundled-flow',
    title: 'Bundled flow',
    description: 'From the bundle.',
    goal: 'Keep metadata.',
    inputs: [],
    nodes: {},
    metadata,
  }

  const flowSaveRequests = (fetchMock: ReturnType<typeof vi.fn>) => fetchMock.mock.calls.filter(([url, init]) =>
    String(url).includes('/api/flows') && (init as RequestInit | undefined)?.method === 'POST')

  beforeEach(() => {
    resetEditorState('bundled-flow')
    resetFlowSaveBaselines()
    useStore.setState({
      editorMode: 'structured',
      clientPreferencesLoaded: true,
      activeProjectPath: '/tmp/project',
      suppressPreview: false,
    })
    // The bundled fallback: the project holds no copy of this flow.
    vi.mocked(loadEditorFlowPayload).mockReset()
    vi.mocked(loadEditorFlowPayload).mockResolvedValue({
      name: 'bundled-flow',
      layer: 'bundled',
      files: [{ name: 'flow.md', content: '---\nid: bundled-flow\n---\n' }],
      flow,
    })
    vi.mocked(loadEditorPreview).mockReset()
    vi.mocked(loadEditorPreview).mockImplementation(async (previewFlow) => ({
      status: 'ok',
      graph: { nodes: [], edges: [], metadata: previewFlow?.metadata ?? {} },
      flow: previewFlow,
      diagnostics: [],
      errors: [],
    }))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('switches an unedited flow to Files without saving, then saves a real edit with the metadata intact', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ status: 'saved', name: 'bundled-flow' }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    renderEditor()
    await waitFor(() => expect(useStore.getState().flowMetadata.software_development).toBeDefined())

    await user.click(screen.getByRole('button', { name: 'Files' }))
    await screen.findByTestId('flow-file-editor')

    expect(flowSaveRequests(fetchMock)).toEqual([])
    expect(useStore.getState().flowMetadata).toMatchObject(metadata)

    await user.click(screen.getByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    act(() => useStore.getState().updateFlowMetadata('title', 'Edited title'))
    await user.click(screen.getByRole('button', { name: 'Files' }))
    await screen.findByTestId('flow-file-editor')

    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    const [, init] = flowSaveRequests(fetchMock)[0]
    const body = JSON.parse(String((init as RequestInit).body))
    expect(body.flow.title).toBe('Edited title')
    expect(body.flow.metadata).toEqual(metadata)
    expect(useStore.getState().flowMetadata).toMatchObject(metadata)
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

  it('keeps edits typed while the save before leaving the files runs, and stays in the files', async () => {
    const user = userEvent.setup()
    let finishSave: (response: Response) => void = () => {}
    const fetchMock = vi.fn(() => new Promise<Response>((resolve) => { finishSave = resolve }))
    vi.stubGlobal('fetch', fetchMock)
    renderEditor()
    const editor = await screen.findByTestId('flow-file-editor')
    const first = '---\nid: demo\n---\nFirst edit.\n'
    const later = '---\nid: demo\n---\nFirst edit. And a later one.\n'
    fireEvent.change(editor, { target: { value: first } })

    await user.click(screen.getByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))
    fireEvent.change(screen.getByTestId('flow-file-editor'), { target: { value: later } })
    vi.mocked(loadEditorFlowPayload).mockResolvedValue({
      name: 'demo', layer: 'org', files: [{ name: 'flow.md', content: first }, files[1]], flow: { id: 'demo' },
    })
    await act(async () => finishSave(new Response(JSON.stringify({ status: 'saved', name: 'demo' }), { status: 200 })))

    await waitFor(() => expect(loadEditorFlowPayload).toHaveBeenCalledTimes(2))
    expect(useStore.getState().editorMode).toBe('files')
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(later)
    expect(screen.getByTestId('flow-files-save')).toBeEnabled()
  })

  it('adds and deletes node files, sending both in one save', async () => {
    const user = userEvent.setup()
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ status: 'saved', name: 'demo' }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    vi.spyOn(window, 'prompt').mockReturnValue('Review step')
    renderEditor()
    await screen.findByTestId('flow-file-editor')
    // flow.md is the flow itself, so it can't be deleted.
    expect(screen.queryByTestId('flow-file-delete')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'node_1.md' }))
    await user.click(screen.getByTestId('flow-file-delete'))
    expect(screen.getByTestId('flow-file-deleted')).toHaveTextContent('node_1.md is deleted when you save.')
    await user.click(screen.getByTestId('flow-file-add'))
    const added = screen.getByTestId('flow-file-editor')
    expect(added).toHaveAccessibleName('review_step.md')
    expect((added as HTMLTextAreaElement).value).toContain('title: Review step')
    expect(screen.getAllByTestId('flow-file').map((row) => [row.getAttribute('data-file'), row.textContent])).toEqual([
      ['flow.md', 'flow.md'],
      ['node_1.md', 'node_1.mddeleted'],
      ['review_step.md', 'review_step.mdnew'],
    ])

    await user.click(screen.getByTestId('flow-files-save'))

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(JSON.parse(String(init.body))).toEqual({
      name: 'demo',
      project_path: '/tmp/project',
      files: [{ name: 'review_step.md', content: (added as HTMLTextAreaElement).value }],
      deleted_files: ['node_1.md'],
    })
  })

  it('drops a new file not saved yet, and restores a file marked for deletion', async () => {
    const user = userEvent.setup()
    vi.spyOn(window, 'prompt').mockReturnValue('node_1')
    renderEditor()
    await screen.findByTestId('flow-file-editor')

    // A taken name gets a number instead of replacing the file.
    await user.click(screen.getByTestId('flow-file-add'))
    expect(screen.getByTestId('flow-file-editor')).toHaveAccessibleName('node_1_2.md')
    await user.click(screen.getByRole('button', { name: 'Discard new file' }))
    expect(screen.getAllByTestId('flow-file').map((row) => row.getAttribute('data-file'))).toEqual(['flow.md', 'node_1.md'])

    await user.click(screen.getByRole('button', { name: 'node_1.md' }))
    await user.click(screen.getByTestId('flow-file-delete'))
    await user.click(screen.getByRole('button', { name: 'Restore' }))
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(files[1].content)
    expect(screen.getByTestId('flow-files-save')).toBeDisabled()
  })

  it('links a refused save’s problems to their files, and clears them once the edit is put back', async () => {
    const user = userEvent.setup()
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({
      detail: {
        status: 'validation_error',
        error: 'flow definition validation failed with 1 error(s): node_1.md: agent_task nodes need at least one route',
        diagnostics: [{ message: 'node_1.md: agent_task nodes need at least one route', file: 'node_1.md' }],
      },
    }), { status: 422 })))
    renderEditor()
    await screen.findByTestId('flow-file-editor')
    await user.click(screen.getByRole('button', { name: 'node_1.md' }))
    fireEvent.change(screen.getByTestId('flow-file-editor'), { target: { value: '---\nkind: agent_task\n---\nPrompt 2\n' } })
    await user.click(screen.getByTestId('flow-files-save'))

    const error = await screen.findByTestId('flow-files-error')
    expect(error).toHaveTextContent('node_1.md: agent_task nodes need at least one route.')
    await user.click(screen.getByRole('button', { name: 'flow.md' }))
    await user.click(screen.getByRole('button', { name: 'Open node_1.md' }))
    expect(screen.getByTestId('flow-file-editor')).toHaveAccessibleName('node_1.md')
    expect(useStore.getState().saveState).toBe('error')

    fireEvent.change(screen.getByTestId('flow-file-editor'), { target: { value: files[1].content } })
    await waitFor(() => expect(screen.queryByTestId('flow-files-error')).not.toBeInTheDocument())
    expect(useStore.getState().saveState).toBe('idle')
    expect(screen.getByTestId('flow-files-save')).toBeDisabled()
  })

  it('opens a broken flow’s files on the file a problem names', async () => {
    const user = userEvent.setup()
    vi.mocked(loadEditorFlowPayload).mockRejectedValue(new ApiHttpError('/attractor/api/flows/{name}', 422, 'invalid', {
      detail: {
        status: 'validation_error',
        error: 'invalid',
        diagnostics: [
          { message: "node 'node_1' is unreachable: no route leads to it from the start node", file: 'node_1.md' },
          { message: 'flow has no start node', file: null },
        ],
      },
    }))
    vi.mocked(loadEditorFlowFiles).mockResolvedValue(files)
    useStore.setState({ preferredEditorMode: 'structured' })
    renderEditor()

    const failed = await screen.findByTestId('editor-flow-load-failed')
    expect(failed).toHaveTextContent('it has 2 problems')
    expect(failed).toHaveTextContent('Flow has no start node.')
    await user.click(await screen.findByRole('button', { name: 'Open node_1.md' }))
    expect(await screen.findByTestId('flow-file-editor')).toHaveAccessibleName('node_1.md')
    expect(screen.getByTestId('flow-files-load-problems')).toHaveTextContent("node_1.md: Node 'node_1' is unreachable")
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
