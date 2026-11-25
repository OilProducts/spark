import { Editor } from '@/features/editor/Editor'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { EditorPreviewResponse } from '@/features/editor/services/editorPreview'
import { refusedFlowSaves, resetFlowSaveBaselines, retryLastSaveContent, retryRefusedFlowSave, saveFlowContent } from '@/lib/flowPersistence'
import { RefusedFlowSaveNotices } from '@/app/AppSessionControllers'
import { useSettingsNavigationProtection } from '@/features/settings/hooks/useSettingsNavigationProtection'
import { ApiHttpError } from '@/lib/attractorClient'
import { switchEnvironment } from '@/test/environments'
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
        kind: 'subflow',
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
    const budget = screen.getByTestId('canvas-interaction-performance-budget')
    expect(budget).toBeVisible()
    expect(budget).toHaveTextContent(/\b16ms\b/)
    expect(profile).toHaveAttribute('data-only-render-visible-elements', 'false')
    expect(profile).toHaveAttribute('data-preview-debounce-ms', '300')

    act(() => {
      previewDeferreds.get('flow-a.dot')?.resolve(buildPreview(25))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '25'))
    expect(profile).toHaveAttribute('data-only-render-visible-elements', 'true')
    expect(profile).toHaveAttribute('data-preview-debounce-ms', '600')

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

    expect(screen.getByRole('button', { name: 'Add step' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Expanded' }))

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Read-only preview. Switch to Parent to edit.')
    expect(screen.queryByRole('button', { name: 'Add step' })).not.toBeInTheDocument()
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
    expect(screen.getByRole('button', { name: 'Add step' })).toBeInTheDocument()
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
    expect(screen.queryByRole('button', { name: 'Add step' })).not.toBeInTheDocument()
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
    const preview = createDeferred<EditorPreviewResponse>()
    vi.mocked(loadEditorPreview).mockReturnValueOnce(preview.promise)
    renderEditor()
    await waitFor(() => expect(loadEditorPreview).toHaveBeenCalled())

    const filesButton = screen.getByRole('button', { name: 'Files' })
    expect(filesButton).toBeDisabled()
    await user.click(filesButton)
    expect(useStore.getState().editorMode).toBe('structured')
    expect(flowSaveRequests(fetchMock)).toEqual([])

    await act(async () => preview.resolve({ ...buildPreview(0), graph: { nodes: [], edges: [], metadata }, flow }))
    await waitFor(() => expect(filesButton).toBeEnabled())
    expect(screen.queryByTestId('canvas-interaction-performance-budget')).not.toBeInTheDocument()
    expect(screen.queryByTestId('canvas-performance-profile')).not.toBeInTheDocument()
    await user.click(filesButton)
    await screen.findByTestId('flow-file-editor')

    expect(flowSaveRequests(fetchMock)).toEqual([])
    expect(useStore.getState().flowMetadata).toMatchObject(metadata)

    await user.click(screen.getByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    expect(flowSaveRequests(fetchMock)).toEqual([])
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

  it('leaves for the files only in the environment the click was made in', async () => {
    const user = userEvent.setup()
    const save = createDeferred<Response>()
    const fetchMock = vi.fn((url: string, init?: RequestInit) => (String(url).endsWith('/api/flows') && init?.method === 'POST'
      ? save.promise
      : Promise.resolve(new Response(JSON.stringify({ status: 'ok' }), { status: 200 }))))
    vi.stubGlobal('fetch', fetchMock)
    await switchEnvironment('env-r')
    const { unmount } = renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    await user.click(await screen.findByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    act(() => useStore.getState().updateFlowMetadata('title', 'Edited title'))
    const completed = vi.fn()
    window.addEventListener('spark:preferences-completed', completed)
    try {
      // The click waits on the remote save; the switch happens meanwhile, as the app does it.
      await user.click(filesButton)
      await waitFor(() => expect(flowSaveRequests(fetchMock)).toHaveLength(1))
      await switchEnvironment(null)
      resetStoreForEnvironmentSwitch(true)
      unmount()
      await act(async () => save.resolve(new Response(JSON.stringify({ status: 'saved', name: 'bundled-flow' }), { status: 200 })))

      expect(flowSaveRequests(fetchMock).map(([url]) => String(url))).toEqual([
        '/workspace/api/environments/links/env-r/proxy/attractor/api/flows',
      ])
      expect(useStore.getState()).toMatchObject({ editorMode: 'structured', preferredEditorMode: 'structured' })
      expect(completed).not.toHaveBeenCalled()
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
  })

  // A fake server for the flow. Its file holds the title and `server.note`, text the flow doesn't parse,
  // as a files edit can leave it; saves land in `server.flow` once `server.held`, if set, answers them,
  // loads wait for `server.loading`, save previews (which answer the flow sent) for `server.previewing`,
  // and reads of the flow for `server.reading`.
  const serveFlow = () => {
    const server: { flow: typeof flow; note: string; held: Deferred<Response> | null; loading: Promise<void> | null; previewing: Promise<void> | null; reading: Promise<void> | null } = { flow, note: '', held: null, loading: null, previewing: null, reading: null }
    const payload = (name: string) => ({
      name, layer: 'bundled' as const, flow: server.flow,
      files: [{ name: 'flow.md', content: `---\nid: ${name}\ntitle: ${server.flow.title}\n---\n${server.note}` }],
    })
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (name: string) => {
      await server.loading
      return payload(name)
    })
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (init?.method !== 'POST') {
        await server.reading
        return new Response(JSON.stringify(payload('bundled-flow')), { status: 200 })
      }
      if (String(url).endsWith('/preview')) {
        await server.previewing
        return new Response(JSON.stringify({ status: 'ok', flow: JSON.parse(String(init.body)).flow }), { status: 200 })
      }
      const response = server.held ? await server.held.promise : new Response(JSON.stringify({ status: 'saved', name: 'bundled-flow' }), { status: 200 })
      if (response.ok) server.flow = JSON.parse(String(init.body)).flow
      return response
    })
    vi.stubGlobal('fetch', fetchMock)
    return { server, fetchMock }
  }
  const unavailable = () => new Response(JSON.stringify({ detail: 'unavailable' }), { status: 503 })
  const CHANGED = 'It may have changed since, so these edits weren’t saved over it.'

  // Opens the remote flow, makes `edit` and leaves for the other view, which saves it; the save is held
  // until `server.held` answers, and the user switches to local meanwhile.
  const sendRemoteEditThenSwitchAway = async (user: ReturnType<typeof userEvent.setup>, server: ReturnType<typeof serveFlow>['server'], edit: string | (() => Promise<void>)) => {
    await switchEnvironment('env-r')
    const remote = renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    if (typeof edit === 'string') {
      await user.click(await screen.findByRole('button', { name: 'Structured' }))
      await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
      act(() => useStore.getState().updateFlowMetadata('title', edit))
    } else {
      await edit()
    }
    server.held = createDeferred<Response>()
    const held = server.held
    await user.click(screen.getByRole('button', { name: typeof edit === 'string' ? 'Files' : 'Structured' }))
    await waitFor(() => expect(vi.mocked(fetch).mock.calls.some(([url, init]) => String(url).endsWith('/api/flows') && init?.method === 'POST')).toBe(true))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remote.unmount()
    server.held = null
    return held
  }

  const returnToRemote = async (activeFlow: string | null) => {
    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    useStore.setState({ viewMode: activeFlow ? 'editor' : 'home', activeProjectPath: '/tmp/project', activeFlow, clientPreferencesLoaded: true })
  }

  // Back in the remote environment, the flow opens again with the notices shown.
  const reopenRemote = async () => {
    await returnToRemote('bundled-flow')
    render(<RefusedFlowSaveNotices />)
    renderEditor()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
  }

  it('keeps a remote flow’s save refused late, once another flow is open there, off that flow, reporting it for its own', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')

    // Back there, another flow opens before the save answers.
    await returnToRemote('other-flow')
    render(<RefusedFlowSaveNotices />)
    renderEditor()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
    await act(async () => held.resolve(unavailable()))

    expect(useStore.getState()).toMatchObject({ activeFlow: 'other-flow', saveState: 'idle', saveErrorMessage: null })
    expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()
    await act(async () => { await retryLastSaveContent() })
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('Couldn’t save bundled-flow')
    expect(notice).toHaveTextContent('unavailable Open the flow to try again.')
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
  })

  it('reports a save refused while its flow opens again through the flow’s notice, never the reopened editor’s Retry', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')

    // The same flow opens again; the save fails while it loads.
    const loading = createDeferred<void>()
    server.loading = loading.promise
    await returnToRemote('bundled-flow')
    render(<RefusedFlowSaveNotices />)
    const loads = vi.mocked(loadEditorFlowPayload).mock.calls.length
    renderEditor()
    await waitFor(() => expect(loadEditorFlowPayload).toHaveBeenCalledTimes(loads + 1))
    await act(async () => held.resolve(unavailable()))
    await act(async () => loading.resolve())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())

    expect(useStore.getState()).toMatchObject({ saveState: 'idle', saveErrorMessage: null, flowMetadata: { title: 'Bundled flow' } })
    expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()
    await act(async () => { await retryLastSaveContent() })
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(screen.getByTestId('refused-flow-save')).toHaveTextContent('unavailable It is unchanged since: Retry saves these edits.')
  })

  it('reports a save refused after a switch away on return, and retries it once the flow opens unchanged', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    // Nothing shows here, in local.
    const local = render(<RefusedFlowSaveNotices />)
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    local.unmount()

    await returnToRemote(null)
    render(<RefusedFlowSaveNotices />)
    expect(screen.getByTestId('refused-flow-save')).toHaveTextContent('unavailable Open the flow to try again.')

    // Opened as the edits found it, Retry saves them, and the flow shows them saved.
    act(() => useStore.setState({ viewMode: 'editor', activeFlow: 'bundled-flow' }))
    renderEditor()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
    expect(useStore.getState().flowMetadata.title).toBe('Bundled flow')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('It is unchanged since: Retry saves these edits.')
    // Refused again, the edits stay with the notice, never the editor's own save state and Retry.
    server.held = createDeferred<Response>()
    server.held.resolve(new Response(JSON.stringify({ detail: 'still unavailable' }), { status: 503 }))
    await user.click(within(notice).getByRole('button', { name: 'Retry save' }))
    await waitFor(() => expect(notice).toHaveTextContent('still unavailable It is unchanged since: Retry saves these edits.'))
    expect(useStore.getState()).toMatchObject({ saveState: 'idle', saveErrorMessage: null })
    expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()
    server.held = null
    await user.click(within(notice).getByRole('button', { name: 'Retry save' }))
    await waitFor(() => expect(useStore.getState().flowMetadata.title).toBe('Unsaved title'))
    expect(server.flow.title).toBe('Unsaved title')
    expect(flowSaveRequests(fetchMock)).toHaveLength(3)
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('reports a save refused after a switch away on return, and never retries it over a version saved since', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    // Meanwhile the flow was saved with newer content there.
    server.flow = { ...flow, title: 'Newer title' }

    await reopenRemote()
    expect(useStore.getState()).toMatchObject({ saveState: 'idle', flowMetadata: { title: 'Newer title' } })
    expect(vi.mocked(loadEditorPreview).mock.calls.map(([previewFlow]) => previewFlow?.title)).not.toContain('Unsaved title')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent(CHANGED)
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()
    await act(async () => { await retryLastSaveContent() })

    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Newer title')
  })

  it('never retries a refused save over a files edit that left the flow’s structure as it was', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    // A files edit there changed only text the flow doesn't parse.
    server.note = 'A note in the flow file.\n'

    await reopenRemote()
    expect(useStore.getState().flowMetadata.title).toBe('Bundled flow')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent(CHANGED)
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    await act(async () => { await retryRefusedFlowSave(refusedFlowSaves()[0]) })
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    expect(server.note).toBe('A note in the flow file.\n')
  })

  it('never retries a refused save over a newer save in flight from the editor the flow opened in', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')
    expect(within(notice).getByRole('button', { name: 'Retry save' })).toBeInTheDocument()
    const [refused] = refusedFlowSaves()

    // A newer edit's save is in flight when the refused one is retried.
    await user.click(screen.getByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    act(() => useStore.getState().updateFlowMetadata('title', 'Newer title'))
    server.held = createDeferred<Response>()
    const newer = server.held
    await user.click(screen.getByRole('button', { name: 'Files' }))
    await waitFor(() => expect(flowSaveRequests(fetchMock)).toHaveLength(2))
    server.held = null
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    const retried = retryRefusedFlowSave(refused)
    await act(async () => newer.resolve(new Response(JSON.stringify({ status: 'saved', name: 'bundled-flow' }), { status: 200 })))

    await expect(retried).resolves.toBe(false)
    expect(flowSaveRequests(fetchMock)).toHaveLength(2)
    expect(server.flow.title).toBe('Newer title')
    expect(notice).toHaveTextContent(CHANGED)
    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('never retries a refused save over one another editor made after the flow opened', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('It is unchanged since: Retry saves these edits.')

    // Another editor saves the flow, then Retry is clicked here.
    server.flow = { ...flow, title: 'Saved elsewhere' }
    await user.click(within(notice).getByRole('button', { name: 'Retry save' }))

    await waitFor(() => expect(notice).toHaveTextContent(CHANGED))
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Saved elsewhere')
    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('opens files edits refused as invalid out of sight back in the files to correct, once the flow opens unchanged, without sending them', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const invalid = '---\nid: bundled-flow\ntitle: Bundled flow\nsteps: [\n---\n'
    const held = await sendRemoteEditThenSwitchAway(user, server, async () => {
      fireEvent.change(await screen.findByTestId('flow-file-editor'), { target: { value: invalid } })
    })
    await act(async () => held.resolve(new Response(JSON.stringify({ detail: { status: 'validation_error', error: 'Steps must be a list.' } }), { status: 422 })))

    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('Steps must be a list. It is unchanged since: Edit files opens these edits to correct them.')
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    await user.click(within(notice).getByRole('button', { name: 'Edit files' }))

    await waitFor(() => expect(useStore.getState().editorMode).toBe('files'))
    expect(await screen.findByTestId('flow-file-editor')).toHaveValue(invalid)
    expect(screen.getByTestId('flow-files-error')).toHaveTextContent('Steps must be a list.')
    expect(screen.getByTestId('flow-files-save')).toBeEnabled()
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
  })

  it('never retries a refused save over one another editor saved while its preview was answering', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(unavailable()))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')
    const previews = () => fetchMock.mock.calls.filter(([url]) => String(url).endsWith('/preview')).length
    const before = previews()

    // Retry's preview is still answering when another editor saves the flow.
    const previewing = createDeferred<void>()
    server.previewing = previewing.promise
    await user.click(within(notice).getByRole('button', { name: 'Retry save' }))
    await waitFor(() => expect(previews()).toBe(before + 1))
    server.flow = { ...flow, title: 'Saved elsewhere' }
    await act(async () => previewing.resolve())

    await waitFor(() => expect(notice).toHaveTextContent(CHANGED))
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Saved elsewhere')
    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('keeps file drafts typed in the reopened editor over files edits refused before it', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, async () => {
      fireEvent.change(await screen.findByTestId('flow-file-editor'), { target: { value: '---\nid: bundled-flow\nsteps: [\n---\n' } })
    })
    await act(async () => held.resolve(new Response(JSON.stringify({ detail: { status: 'validation_error', error: 'Steps must be a list.' } }), { status: 422 })))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')
    expect(within(notice).getByRole('button', { name: 'Edit files' })).toBeInTheDocument()
    const [refused] = refusedFlowSaves()

    // A newer draft typed here makes the flow changed since: the older edits never replace it.
    await user.click(screen.getByRole('button', { name: 'Files' }))
    const newer = '---\nid: bundled-flow\ntitle: Newer draft\n---\n'
    fireEvent.change(await screen.findByTestId('flow-file-editor'), { target: { value: newer } })
    await waitFor(() => expect(notice).toHaveTextContent(CHANGED))
    expect(within(notice).queryByRole('button', { name: 'Edit files' })).not.toBeInTheDocument()
    await act(async () => { await expect(retryRefusedFlowSave(refused)).resolves.toBe(false) })
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(newer)
    expect(screen.getByTestId('refused-flow-save')).toBe(notice)
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
  })

  it('recovers edits refused after a switch away over the files an earlier autosave in the same opening left', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    await switchEnvironment('env-r')
    const remote = renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    await user.click(await screen.findByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    // An autosave lands, as the flow settings send one; then a later edit's save is still answering
    // when the user switches away.
    await act(async () => { await expect(saveFlowContent('bundled-flow', { flow: { ...flow, title: 'Saved title' } })).resolves.toBe(true) })
    expect(server.flow.title).toBe('Saved title')
    server.held = createDeferred<Response>()
    const held = server.held
    act(() => useStore.getState().updateFlowMetadata('title', 'Unsaved title'))
    await user.click(filesButton)
    await waitFor(() => expect(flowSaveRequests(fetchMock)).toHaveLength(2))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remote.unmount()
    server.held = null
    await act(async () => held.resolve(unavailable()))

    await reopenRemote()
    expect(useStore.getState().flowMetadata.title).toBe('Saved title')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('unavailable It is unchanged since: Retry saves these edits.')
    await user.click(within(notice).getByRole('button', { name: 'Retry save' }))
    await waitFor(() => expect(useStore.getState().flowMetadata.title).toBe('Unsaved title'))
    expect(server.flow.title).toBe('Unsaved title')
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('opens flow edits refused as invalid out of sight back on the canvas to correct, once the flow opens unchanged, without sending them', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(new Response(JSON.stringify({ detail: { status: 'validation_error', error: 'Titles are short.' } }), { status: 422 })))

    await reopenRemote()
    expect(useStore.getState().flowMetadata.title).toBe('Bundled flow')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent('Titles are short. It is unchanged since: Edit flow opens these edits to correct them.')
    await user.click(within(notice).getByRole('button', { name: 'Edit flow' }))

    await waitFor(() => expect(useStore.getState().flowMetadata.title).toBe('Unsaved title'))
    expect(useStore.getState()).toMatchObject({ editorMode: 'structured', saveState: 'error', saveErrorMessage: expect.stringContaining('Titles are short.') })
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Bundled flow')
    // They stay unsaved: the next save sends them, as it would once corrected.
    await user.click(screen.getByRole('button', { name: 'Files' }))
    await waitFor(() => expect(server.flow.title).toBe('Unsaved title'))
    expect(useStore.getState().saveState).not.toBe('error')
  })

  it('reports an autosave refused behind another view of its still-open editor, for that flow and project', async () => {
    const user = userEvent.setup()
    const { server } = serveFlow()
    render(<RefusedFlowSaveNotices />)
    renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    await user.click(await screen.findByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    // The user opens Runs; an autosave of the flow settings, still waiting, goes then and is refused.
    const refuseAutosaveBehindRuns = async () => {
      act(() => useStore.setState({ viewMode: 'runs' }))
      server.held = createDeferred<Response>()
      server.held.resolve(unavailable())
      await act(async () => { await expect(saveFlowContent('bundled-flow', { flow: { ...flow, title: 'Unsaved title' } })).resolves.toBe(false) })
      server.held = null
      const notice = screen.getByTestId('refused-flow-save')
      expect(notice).toHaveTextContent('Couldn’t save bundled-flow')
      expect(notice).toHaveTextContent('unavailable Its edits are still in the editor.')
      expect(refusedFlowSaves()).toMatchObject([{ name: 'bundled-flow', projectPath: '/tmp/project', environmentId: null }])
      return notice
    }
    await user.click(within(await refuseAutosaveBehindRuns()).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()

    // Back in the editor, it shows the failure itself; its Retry saves the edits, and the notice goes with them.
    await refuseAutosaveBehindRuns()
    act(() => useStore.setState({ viewMode: 'editor' }))
    expect(useStore.getState()).toMatchObject({ saveState: 'error', saveErrorMessage: 'unavailable' })
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
    await user.click(screen.getByTestId('editor-save-retry'))
    await waitFor(() => expect(server.flow.title).toBe('Unsaved title'))
    act(() => useStore.setState({ viewMode: 'runs' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('never retries edits refused after an autosave over a version saved elsewhere before the autosave’s files were read back', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    await switchEnvironment('env-r')
    const remote = renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    await user.click(await screen.findByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    // An autosave lands; another client saves before its files are read back.
    const reads = () => fetchMock.mock.calls.filter(([, init]) => init?.method !== 'POST').length
    const before = reads()
    const reading = createDeferred<void>()
    server.reading = reading.promise
    const autosave = saveFlowContent('bundled-flow', { flow: { ...flow, title: 'Saved title' } })
    await waitFor(() => expect(reads()).toBe(before + 1))
    server.flow = { ...flow, title: 'Saved elsewhere' }
    server.reading = null
    await act(async () => { reading.resolve(); await expect(autosave).resolves.toBe(true) })
    // A later edit's save is still answering when the user switches away, and is refused.
    server.held = createDeferred<Response>()
    const held = server.held
    act(() => useStore.getState().updateFlowMetadata('title', 'Unsaved title'))
    await user.click(filesButton)
    await waitFor(() => expect(flowSaveRequests(fetchMock)).toHaveLength(2))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remote.unmount()
    server.held = null
    await act(async () => held.resolve(unavailable()))

    await reopenRemote()
    expect(useStore.getState().flowMetadata.title).toBe('Saved elsewhere')
    const notice = screen.getByTestId('refused-flow-save')
    expect(notice).toHaveTextContent(`unavailable ${CHANGED}`)
    expect(within(notice).queryByRole('button', { name: 'Retry save' })).not.toBeInTheDocument()
    await act(async () => { await expect(retryRefusedFlowSave(refusedFlowSaves()[0])).resolves.toBe(false) })
    expect(flowSaveRequests(fetchMock)).toHaveLength(2)
    expect(server.flow.title).toBe('Saved elsewhere')
    await user.click(within(notice).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('opens a version saved while Edit flow opens refused edits, keeping them for their notice', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(new Response(JSON.stringify({ detail: { status: 'validation_error', error: 'Titles are short.' } }), { status: 422 })))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')

    // The flow is saved elsewhere while the editor loads it to open the edits.
    const loading = createDeferred<void>()
    server.loading = loading.promise
    const loads = vi.mocked(loadEditorFlowPayload).mock.calls.length
    const previews = vi.mocked(loadEditorPreview).mock.calls.length
    await user.click(within(notice).getByRole('button', { name: 'Edit flow' }))
    await waitFor(() => expect(loadEditorFlowPayload).toHaveBeenCalledTimes(loads + 1))
    server.flow = { ...flow, title: 'Saved elsewhere' }
    server.loading = null
    await act(async () => loading.resolve())

    await waitFor(() => expect(useStore.getState().flowMetadata.title).toBe('Saved elsewhere'))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
    expect(useStore.getState()).toMatchObject({ saveState: 'idle', saveErrorMessage: null })
    expect(screen.getByTestId('refused-flow-save')).toHaveTextContent(`Titles are short. ${CHANGED}`)
    expect(within(screen.getByTestId('refused-flow-save')).queryByRole('button', { name: 'Edit flow' })).not.toBeInTheDocument()
    // The refused edits never reached the canvas: leaving it saves nothing over the newer version.
    expect(vi.mocked(loadEditorPreview).mock.calls.slice(previews).map(([previewFlow]) => previewFlow?.title)).not.toContain('Unsaved title')
    await user.click(screen.getByRole('button', { name: 'Files' }))
    await screen.findByTestId('flow-file-editor')
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Saved elsewhere')
  })

  it('keeps the flow usable as saved, and refused edits to copy, when the canvas can’t show them', async () => {
    const user = userEvent.setup()
    const { server, fetchMock } = serveFlow()
    const held = await sendRemoteEditThenSwitchAway(user, server, 'Unsaved title')
    await act(async () => held.resolve(new Response(JSON.stringify({ detail: { status: 'validation_error', error: 'Step review isn’t wired.' } }), { status: 422 })))
    await reopenRemote()
    const notice = screen.getByTestId('refused-flow-save')

    // Their preview has problems but no graph to show them by.
    const previewOf = vi.mocked(loadEditorPreview).getMockImplementation()!
    vi.mocked(loadEditorPreview).mockImplementation(async (previewFlow, ...rest) => (previewFlow?.title === 'Unsaved title'
      ? { status: 'validation_error', error: 'Step review isn’t wired.', diagnostics: [], errors: [] }
      : previewOf(previewFlow, ...rest)))
    await user.click(within(notice).getByRole('button', { name: 'Edit flow' }))

    await waitFor(() => expect(screen.getByTestId('refused-flow-save')).toHaveTextContent('The canvas can’t show these edits, so the flow opened as saved; copy them from here.'))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
    expect(useStore.getState()).toMatchObject({ saveState: 'idle', flowMetadata: { title: 'Bundled flow' } })
    const kept = screen.getByTestId('refused-flow-save')
    expect(within(kept).queryByRole('button', { name: 'Edit flow' })).not.toBeInTheDocument()
    expect(JSON.parse(within(kept).getByTestId('refused-flow-save-edits').textContent ?? '')).toMatchObject({ title: 'Unsaved title' })
    expect(flowSaveRequests(fetchMock)).toHaveLength(1)
    expect(server.flow.title).toBe('Bundled flow')
    await user.click(within(kept).getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByTestId('refused-flow-save')).not.toBeInTheDocument()
  })

  it('keeps a valid local flow runnable when a remote flow’s preview answers after a switch here', async () => {
    const remotePreview = createDeferred<EditorPreviewResponse>()
    let held = false
    vi.mocked(loadEditorPreview).mockImplementation(async (previewFlow, init) => {
      // The first preview the editor asks for on its own, after loading, is the remote one held.
      if (!held && init === undefined) {
        held = true
        return remotePreview.promise
      }
      return { status: 'ok', graph: { nodes: [], edges: [], metadata: previewFlow?.metadata ?? {} }, flow: previewFlow, diagnostics: [], errors: [] } as unknown as EditorPreviewResponse
    })
    await switchEnvironment('env-r')
    const remote = renderEditor()
    await waitFor(() => expect(held).toBe(true))

    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remote.unmount()
    useStore.setState({ viewMode: 'editor', activeProjectPath: '/tmp/project', activeFlow: 'bundled-flow', clientPreferencesLoaded: true })
    renderEditor()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Files' })).toBeEnabled())
    await waitFor(() => expect(vi.mocked(loadEditorPreview).mock.calls.filter(([, init]) => init === undefined)).toHaveLength(2))
    expect(useStore.getState().hasValidationErrors).toBe(false)

    await act(async () => remotePreview.resolve({ status: 'ok', graph: { nodes: [], edges: [], graph_attrs: {} }, diagnostics: [
      { rule_id: 'decision_domain', severity: 'error', message: 'review.md: agent_task steps need at least one route', node_id: 'review' },
    ], errors: [] } as unknown as EditorPreviewResponse))
    expect(useStore.getState()).toMatchObject({ hasValidationErrors: false, diagnostics: [] })
  })

  it.each([
    ['saved', new Response(JSON.stringify({ status: 'saved', name: 'bundled-flow' }), { status: 200 })],
    ['refused', new Response(JSON.stringify({ detail: 'unavailable' }), { status: 503 })],
  ])('ends a navigation held on a remote save that is %s after a switch to local Settings with unsaved edits, asking nothing there', async (_, answer) => {
    const user = userEvent.setup()
    const save = createDeferred<Response>()
    const fetchMock = vi.fn((url: string, init?: RequestInit) => (String(url).endsWith('/api/flows') && init?.method === 'POST'
      ? save.promise
      : Promise.resolve(new Response(JSON.stringify({ status: 'ok' }), { status: 200 }))))
    vi.stubGlobal('fetch', fetchMock)
    const ask = vi.spyOn(window, 'confirm').mockReturnValue(true)
    await switchEnvironment('env-r')
    const remote = renderEditor()
    const filesButton = screen.getByRole('button', { name: 'Files' })
    await waitFor(() => expect(filesButton).toBeEnabled())
    await user.click(filesButton)
    await user.click(await screen.findByRole('button', { name: 'Structured' }))
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    act(() => useStore.getState().updateFlowMetadata('title', 'Remote title'))
    await user.click(filesButton)
    await waitFor(() => expect(flowSaveRequests(fetchMock)).toHaveLength(1))
    // Opening another flow waits on the remote save; the user switches to local Settings and edits them.
    act(() => useStore.getState().setActiveFlow('other-flow'))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remote.unmount()
    act(() => useStore.setState({ viewMode: 'settings', activeFlow: 'local-flow' }))
    const LocalSettingsDraft = () => { useSettingsNavigationProtection(true); return null }
    render(<LocalSettingsDraft />)

    await act(async () => save.resolve(answer))
    await act(async () => {})
    expect(ask).not.toHaveBeenCalled()
    expect(useStore.getState()).toMatchObject({ viewMode: 'settings', activeFlow: 'local-flow', saveState: 'idle' })
    expect(flowSaveRequests(fetchMock).map(([url]) => String(url))).toEqual([
      '/workspace/api/environments/links/env-r/proxy/attractor/api/flows',
    ])
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
    // The switch says it is saving, and can't be started again meanwhile.
    expect(screen.getByTestId('editor-mode-switching')).toHaveTextContent('Saving…')
    expect(screen.getByRole('button', { name: 'Structured' })).toBeDisabled()
    fireEvent.change(screen.getByTestId('flow-file-editor'), { target: { value: later } })
    vi.mocked(loadEditorFlowPayload).mockResolvedValue({
      name: 'demo', layer: 'org', files: [{ name: 'flow.md', content: first }, files[1]], flow: { id: 'demo' },
    })
    await act(async () => finishSave(new Response(JSON.stringify({ status: 'saved', name: 'demo' }), { status: 200 })))

    await waitFor(() => expect(loadEditorFlowPayload).toHaveBeenCalledTimes(2))
    expect(useStore.getState().editorMode).toBe('files')
    expect(screen.getByTestId('flow-file-editor')).toHaveValue(later)
    expect(screen.getByTestId('flow-files-save')).toBeEnabled()
    expect(screen.queryByTestId('editor-mode-switching')).not.toBeInTheDocument()
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
        error: 'flow definition validation failed with 1 error(s): node_1.md: agent_task steps need at least one route',
        diagnostics: [{ message: 'node_1.md: agent_task steps need at least one route', file: 'node_1.md' }],
      },
    }), { status: 422 })))
    renderEditor()
    await screen.findByTestId('flow-file-editor')
    await user.click(screen.getByRole('button', { name: 'node_1.md' }))
    fireEvent.change(screen.getByTestId('flow-file-editor'), { target: { value: '---\nkind: agent_task\n---\nPrompt 2\n' } })
    await user.click(screen.getByTestId('flow-files-save'))

    const error = await screen.findByTestId('flow-files-error')
    expect(error).toHaveTextContent('node_1.md: Agent steps need at least one route.')
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
          { message: "node 'node_1' is unreachable: no route leads to it from the start node", file: 'node_1.md', node_id: 'node_1', node_label: 'Draft Plan' },
          { message: 'flow has no start node', file: null },
        ],
      },
    }))
    vi.mocked(loadEditorFlowFiles).mockResolvedValue(files)
    useStore.setState({ preferredEditorMode: 'structured' })
    renderEditor()

    const failed = await screen.findByTestId('editor-flow-load-failed')
    expect(failed).toHaveTextContent('it has 2 problems')
    expect(failed).toHaveTextContent('Flow has no start step.')
    await user.click(await screen.findByRole('button', { name: 'Open node_1.md' }))
    expect(await screen.findByTestId('flow-file-editor')).toHaveAccessibleName('node_1.md')
    // The step goes by its title, not its ID.
    expect(screen.getByTestId('flow-files-load-problems')).toHaveTextContent('node_1.md: “Draft Plan” is unreachable')
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
    expect(error).toHaveTextContent('Missing id')
    expect(loadEditorFlowPayload).toHaveBeenCalledTimes(1)
    expect(screen.getByTestId('flow-file-editor')).toHaveValue('---\n---\n')
  })
})
