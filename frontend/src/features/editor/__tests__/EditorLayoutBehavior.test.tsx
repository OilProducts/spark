import { Editor } from '@/features/editor/Editor'
import type { EditorPreviewResponse } from '@/features/editor/services/editorPreview'
import {
  loadEditorFlowPayload,
  loadEditorPreview,
} from '@/features/editor/services/editorPreview'
import { buildFlowLayoutFromNodesAndEdges } from '@/lib/flowLayout'
import { EDGE_RENDER_ROUTE_KEY } from '@/lib/flowLayoutConstants'
import { buildSavedFlowLayoutStorageKey, type SavedFlowLayoutV1 } from '@/lib/flowLayoutPersistence'
import { routeFixedNodeGraph, type EdgeRoute } from '@/lib/edgeRouting'
import * as flowPersistence from '@/lib/flowPersistence'
import { useStore } from '@/store'
import { ReactFlowProvider, type Edge, type Node } from '@xyflow/react'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const reactFlowHarness = vi.hoisted(() => ({
  latestProps: null as null | {
    nodes: Node[]
    edges: Edge[]
    onConnect: (params: Record<string, unknown>) => void
    onNodesChange: (changes: Array<Record<string, unknown>>) => void
  },
}))

const elkHarness = vi.hoisted(() => ({
  calls: [] as Array<Record<string, unknown>>,
  layouts: [] as Array<Record<string, { x: number; y: number }>>,
}))

const routerHarness = vi.hoisted(() => ({
  calls: [] as Array<Record<string, unknown>>,
  responders: [] as Array<(request: Record<string, unknown>) => Promise<Record<string, EdgeRoute>>>,
}))

vi.mock('elkjs/lib/elk.bundled.js', () => ({
  default: class MockElk {
    async layout(graph: { children?: Array<{ id: string }> }) {
      elkHarness.calls.push(graph as unknown as Record<string, unknown>)
      const queuedLayout = elkHarness.layouts.shift() ?? {}
      return {
        children: (graph.children ?? []).map((child, index) => ({
          id: child.id,
          x: queuedLayout[child.id]?.x ?? index * 260,
          y: queuedLayout[child.id]?.y ?? 0,
        })),
      }
    }
  },
}))

vi.mock('@/lib/flowLayoutRouterClient', () => ({
  routeFixedNodeGraphInWorker: vi.fn((request: Record<string, unknown>) => {
    routerHarness.calls.push(request)
    const responder = routerHarness.responders.shift()
    if (responder) {
      return responder(request)
    }
    return Promise.resolve(routeFixedNodeGraph(request as never).routes)
  }),
}))

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

vi.mock('@/features/editor/components/ValidationPanel', () => ({
  ValidationPanel: () => null,
}))

vi.mock('@/lib/flowPersistence', async () => {
  const actual = await vi.importActual<typeof import('@/lib/flowPersistence')>('@/lib/flowPersistence')
  return {
    ...actual,
    primeFlowSaveBaseline: vi.fn(),
    saveFlowContent: vi.fn(async () => true),
    saveFlowContentExpectingSemanticEquivalence: vi.fn(async () => true),
  }
})

vi.mock('@xyflow/react', async () => {
  const actual = await vi.importActual<typeof import('@xyflow/react')>('@xyflow/react')

  return {
    ...actual,
    ReactFlowProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
    ReactFlow: (props: {
      nodes: Node[]
      edges: Edge[]
      children?: React.ReactNode
      onConnect: (params: Record<string, unknown>) => void
      onNodesChange: (changes: Array<Record<string, unknown>>) => void
    }) => {
      reactFlowHarness.latestProps = props
      return (
        <div
          data-testid="mock-react-flow"
          data-node-count={String(props.nodes.length)}
          data-edge-count={String(props.edges.length)}
        >
          {props.nodes.map((node) => (
            <div
              key={node.id}
              data-testid={`mock-node-${node.id}`}
              data-x={String(node.position.x)}
              data-y={String(node.position.y)}
            />
          ))}
          {props.edges.map((edge) => (
            <div
              key={edge.id}
              data-testid={`mock-edge-${edge.id}`}
              data-route={JSON.stringify((edge.data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY] ?? null)}
            />
          ))}
          {props.children}
        </div>
      )
    },
    Controls: () => null,
    MiniMap: () => null,
    Background: () => null,
  }
})

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
  request?: Record<string, unknown>
}

const PROJECT_PATH = '/tmp/project-layout-behavior'

const createDeferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const buildPreview = (
  nodeIds: string[],
  edges: Array<{ from: string; to: string }>,
  options?: {
    childPreviews?: Record<string, unknown>
    nodeTypes?: Record<string, string>
  },
): EditorPreviewResponse => ({
  status: 'ok',
  graph: {
    nodes: nodeIds.map((id) => ({
      id,
      label: id.toUpperCase(),
      shape: options?.nodeTypes?.[id] === 'stack.manager_loop' ? 'house' : 'box',
      ...(options?.nodeTypes?.[id] ? { type: options.nodeTypes[id] } : {}),
    })),
    edges,
    graph_attrs: {},
    ...(options?.childPreviews ? { child_previews: options.childPreviews } : {}),
  },
  diagnostics: [],
  errors: [],
})

const buildManagerPreview = (expanded: boolean): EditorPreviewResponse => buildPreview(
  ['start', 'manager'],
  [{ from: 'start', to: 'manager' }],
  {
    nodeTypes: { manager: 'stack.manager_loop' },
    childPreviews: expanded
      ? {
          manager: {
            flow_name: 'child.dot',
            flow_path: '/tmp/child.dot',
            flow_label: 'Child Flow',
            read_only: true,
            provenance: 'derived_child_preview',
            graph: {
              graph_attrs: {},
              nodes: [
                { id: 'child_start', label: 'Child Start', shape: 'Mdiamond' },
                { id: 'child_task', label: 'Child Task', shape: 'box' },
              ],
              edges: [
                { from: 'child_start', to: 'child_task' },
              ],
            },
          },
        }
      : undefined,
  },
)

const resetEditorState = (activeFlow: string | null) => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'editor',
    activeProjectPath: PROJECT_PATH,
    activeFlow,
    executionFlow: null,
    suppressPreview: true,
    editorMode: 'structured',
    rawDotDraft: '',
    rawHandoffError: null,
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

const queueElkLayout = (positions: Record<string, { x: number; y: number }>) => {
  elkHarness.layouts.push(positions)
}

const buildCanvasNodes = (positions: Record<string, { x: number; y: number }>): Node[] =>
  Object.entries(positions).map(([id, position]) => ({
    id,
    type: 'taskNode',
    position,
    width: 220,
    height: 110,
    style: { width: 220, height: 110 },
    data: { label: id.toUpperCase(), shape: 'box' },
  }))

const buildCanvasEdges = (pairs: Array<{ from: string; to: string }>): Edge[] =>
  pairs.map((edge, index) => ({
    id: `e-${edge.from}-${edge.to}-${index}`,
    source: edge.from,
    target: edge.to,
    type: 'validation',
  }))

const buildSavedLayout = (
  positions: Record<string, { x: number; y: number }>,
  pairs: Array<{ from: string; to: string }>,
): SavedFlowLayoutV1 => buildFlowLayoutFromNodesAndEdges(
  buildCanvasNodes(positions),
  buildCanvasEdges(pairs),
).layout

const storageKeyFor = (flowName: string) => buildSavedFlowLayoutStorageKey(
  PROJECT_PATH,
  flowName,
  'editor-parent-only',
)

const readSavedLayout = (flowName: string): SavedFlowLayoutV1 =>
  JSON.parse(window.localStorage.getItem(storageKeyFor(flowName)) ?? 'null') as SavedFlowLayoutV1

const currentNodePosition = (nodeId: string) => {
  const node = reactFlowHarness.latestProps?.nodes.find((entry) => entry.id === nodeId)
  return node ? { x: node.position.x, y: node.position.y } : null
}

const currentEdgeRoute = (edgeId: string) => {
  const edge = reactFlowHarness.latestProps?.edges.find((entry) => entry.id === edgeId)
  return ((edge?.data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY] ?? null) as EdgeRoute | null
}

const queueDeferredRouterResponse = () => {
  const deferred = createDeferred<Record<string, EdgeRoute>>()
  routerHarness.responders.push((request) => {
    deferred.request = request
    return deferred.promise
  })
  return deferred
}

const usePreviewMap = (previews: Record<string, EditorPreviewResponse>) => {
  vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({
    name: flowName,
    content: flowName,
  }))
  vi.mocked(loadEditorPreview).mockImplementation(async (_dot: string, _init, options) => {
    const flowName = options?.flowName ?? 'unknown.dot'
    const preview = previews[flowName]
    if (!preview) {
      throw new Error(`Unexpected preview request for ${flowName}`)
    }
    return preview
  })
}

beforeEach(() => {
  reactFlowHarness.latestProps = null
  elkHarness.calls.length = 0
  elkHarness.layouts.length = 0
  routerHarness.calls.length = 0
  routerHarness.responders.length = 0
  resetEditorState('flow-a.dot')
  vi.clearAllMocks()
  vi.spyOn(console, 'error').mockImplementation(() => {})
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('Editor layout behavior', () => {
  it('loads SavedFlowLayoutV1 from localStorage on first open and persists the latest layout after drag end', async () => {
    const preview = buildPreview(['a', 'b'], [{ from: 'a', to: 'b' }])
    const savedLayout = buildSavedLayout(
      {
        a: { x: 60, y: 80 },
        b: { x: 520, y: 120 },
      },
      [{ from: 'a', to: 'b' }],
    )
    window.localStorage.setItem(storageKeyFor('flow-a.dot'), JSON.stringify(savedLayout))
    usePreviewMap({ 'flow-a.dot': preview })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 300, y: 0 },
    })

    renderEditor()

    await waitFor(() => {
      expect(currentNodePosition('a')).toEqual({ x: 60, y: 80 })
      expect(currentNodePosition('b')).toEqual({ x: 520, y: 120 })
      expect(currentEdgeRoute('e-a-b-0')).toEqual(savedLayout.edgeLayouts['a->b#0'].route)
    })

    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 640, y: 180 },
          dragging: false,
        },
      ])
    })

    await waitFor(() => {
      expect(currentNodePosition('b')).toEqual({ x: 640, y: 180 })
      const persistedLayout = readSavedLayout('flow-a.dot')
      expect(persistedLayout.nodePositions.b).toEqual({ x: 640, y: 180 })
      expect(persistedLayout.edgeLayouts['a->b#0'].route).toEqual(currentEdgeRoute('e-a-b-0'))
    })
  })

  it('reroutes connected and intersecting edges at the throttled cadence, keeps the old route visible, and discards stale worker results', async () => {
    const preview = buildPreview(
      ['a', 'b', 'c', 'd'],
      [
        { from: 'a', to: 'b' },
        { from: 'c', to: 'd' },
      ],
    )
    const savedLayout = buildSavedLayout(
      {
        a: { x: 0, y: 0 },
        b: { x: 340, y: 0 },
        c: { x: 0, y: 320 },
        d: { x: 620, y: 320 },
      },
      [
        { from: 'a', to: 'b' },
        { from: 'c', to: 'd' },
      ],
    )
    savedLayout.edgeLayouts['c->d#0'].route = [
      { x: 220, y: 375 },
      { x: 460, y: 375 },
      { x: 460, y: 60 },
      { x: 340, y: 60 },
    ]
    window.localStorage.setItem(storageKeyFor('flow-a.dot'), JSON.stringify(savedLayout))
    usePreviewMap({ 'flow-a.dot': preview })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 340, y: 0 },
      c: { x: 0, y: 320 },
      d: { x: 620, y: 320 },
    })

    renderEditor()

    await waitFor(() => {
      expect(currentEdgeRoute('e-a-b-0')).toEqual(savedLayout.edgeLayouts['a->b#0'].route)
      expect(currentEdgeRoute('e-c-d-1')).toEqual(savedLayout.edgeLayouts['c->d#0'].route)
    })

    vi.useFakeTimers()
    const firstDeferred = queueDeferredRouterResponse()
    const secondDeferred = queueDeferredRouterResponse()

    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 340, y: 120 },
          dragging: true,
        },
      ])
    })

    expect(routerHarness.calls).toHaveLength(1)
    expect((routerHarness.calls[0].edges as Array<{ id: string }>).map((edge) => edge.id)).toEqual(
      expect.arrayContaining(['a->b#0', 'c->d#0']),
    )
    expect(currentEdgeRoute('e-a-b-0')).toEqual(savedLayout.edgeLayouts['a->b#0'].route)
    expect(currentEdgeRoute('e-c-d-1')).toEqual(savedLayout.edgeLayouts['c->d#0'].route)

    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 340, y: 220 },
          dragging: true,
        },
      ])
      vi.advanceTimersByTime(80)
    })

    expect(routerHarness.calls).toHaveLength(2)
    expect((routerHarness.calls[1].edges as Array<{ id: string }>).map((edge) => edge.id)).toEqual(
      expect.arrayContaining(['a->b#0', 'c->d#0']),
    )
    expect(currentEdgeRoute('e-a-b-0')).toEqual(savedLayout.edgeLayouts['a->b#0'].route)
    expect(currentEdgeRoute('e-c-d-1')).toEqual(savedLayout.edgeLayouts['c->d#0'].route)

    const newerRoutes = {
      'a->b#0': [
        { x: 220, y: 55 },
        { x: 280, y: 55 },
        { x: 280, y: 275 },
      ],
      'c->d#0': [
        { x: 220, y: 375 },
        { x: 520, y: 375 },
        { x: 520, y: 260 },
      ],
    } satisfies Record<string, EdgeRoute>

    await act(async () => {
      secondDeferred.resolve(newerRoutes)
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(currentEdgeRoute('e-a-b-0')).toEqual(newerRoutes['a->b#0'])
    expect(currentEdgeRoute('e-c-d-1')).toEqual(newerRoutes['c->d#0'])

    const staleRoutes = {
      'a->b#0': [
        { x: 220, y: 55 },
        { x: 260, y: 55 },
        { x: 260, y: 175 },
      ],
      'c->d#0': [
        { x: 220, y: 375 },
        { x: 480, y: 375 },
        { x: 480, y: 140 },
      ],
    } satisfies Record<string, EdgeRoute>

    await act(async () => {
      firstDeferred.resolve(staleRoutes)
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(currentEdgeRoute('e-a-b-0')).toEqual(newerRoutes['a->b#0'])
    expect(currentEdgeRoute('e-c-d-1')).toEqual(newerRoutes['c->d#0'])
  })

  it('does not reroute unrelated edges when a drag cannot affect them', async () => {
    const preview = buildPreview(
      ['a', 'b', 'c', 'd'],
      [
        { from: 'a', to: 'b' },
        { from: 'c', to: 'd' },
      ],
    )
    usePreviewMap({ 'flow-a.dot': preview })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 340, y: 0 },
      c: { x: 0, y: 420 },
      d: { x: 340, y: 420 },
    })

    renderEditor()

    await waitFor(() => {
      expect(currentEdgeRoute('e-a-b-0')).not.toBeNull()
      expect(currentEdgeRoute('e-c-d-1')).not.toBeNull()
    })

    const unrelatedBefore = currentEdgeRoute('e-c-d-1')
    const deferred = queueDeferredRouterResponse()

    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 380, y: 0 },
          dragging: true,
        },
      ])
    })

    expect(routerHarness.calls).toHaveLength(1)
    expect((routerHarness.calls[0].edges as Array<{ id: string }>).map((edge) => edge.id)).toEqual(['a->b#0'])

    await act(async () => {
      deferred.resolve({
        'a->b#0': [
          { x: 220, y: 55 },
          { x: 300, y: 55 },
          { x: 300, y: 40 },
        ],
      })
      await Promise.resolve()
    })

    await waitFor(() => {
      expect(currentEdgeRoute('e-c-d-1')).toEqual(unrelatedBefore)
    })
  })

  it('preserves side-handle intent after reconnect, reroute, and reload', async () => {
    let preview = buildPreview(['a', 'b'], [])
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({
      name: flowName,
      content: flowName,
    }))
    vi.mocked(loadEditorPreview).mockImplementation(async (_dot: string, _init, options) => {
      const flowName = options?.flowName ?? 'unknown.dot'
      if (flowName !== 'flow-a.dot') {
        throw new Error(`Unexpected preview request for ${flowName}`)
      }
      return preview
    })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 420, y: 0 },
    })

    const firstRender = renderEditor()

    await waitFor(() => {
      expect(screen.getByTestId('mock-react-flow')).toHaveAttribute('data-edge-count', '0')
    })

    await act(async () => {
      reactFlowHarness.latestProps?.onConnect({
        source: 'a',
        target: 'b',
        sourceHandle: 'source-right',
        targetHandle: 'target-left',
      })
    })

    await waitFor(() => {
      expect(screen.getByTestId('mock-react-flow')).toHaveAttribute('data-edge-count', '1')
      const persistedLayout = readSavedLayout('flow-a.dot')
      expect(persistedLayout.edgeLayouts['a->b#0']).toMatchObject({
        sourceSide: 'right',
        targetSide: 'left',
      })
    })

    preview = buildPreview(['a', 'b'], [{ from: 'a', to: 'b' }])
    firstRender.unmount()
    reactFlowHarness.latestProps = null
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 420, y: 0 },
    })

    renderEditor()

    await waitFor(() => {
      expect(screen.getByTestId('mock-react-flow')).toHaveAttribute('data-edge-count', '1')
    })

    const deferred = queueDeferredRouterResponse()
    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 420, y: 140 },
          dragging: false,
        },
      ])
    })

    await waitFor(() => {
      expect(routerHarness.calls).toHaveLength(2)
    })
    const latestCall = routerHarness.calls[1].edges as Array<{
      id: string
      sourcePort: { side: string }
      targetPort: { side: string }
    }>
    expect(latestCall).toEqual([
      expect.objectContaining({
        id: 'a->b#0',
        sourcePort: expect.objectContaining({ side: 'right' }),
        targetPort: expect.objectContaining({ side: 'left' }),
      }),
    ])

    await act(async () => {
      deferred.resolve({
        'a->b#0': [
          { x: 220, y: 55 },
          { x: 444, y: 55 },
          { x: 444, y: 195 },
        ],
      })
      await Promise.resolve()
    })

    await waitFor(() => {
      const persistedLayout = readSavedLayout('flow-a.dot')
      expect(persistedLayout.edgeLayouts['a->b#0']).toMatchObject({
        sourceSide: 'right',
        targetSide: 'left',
      })
    })
  })

  it('writes fresh layout state for Auto Arrange and clears then rewrites it for Reset Saved Layout', async () => {
    const user = userEvent.setup()
    const preview = buildPreview(['a', 'b'], [{ from: 'a', to: 'b' }])
    const initialSavedLayout = buildSavedLayout(
      {
        a: { x: 80, y: 90 },
        b: { x: 520, y: 120 },
      },
      [{ from: 'a', to: 'b' }],
    )
    window.localStorage.setItem(storageKeyFor('flow-a.dot'), JSON.stringify(initialSavedLayout))
    usePreviewMap({ 'flow-a.dot': preview })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 300, y: 0 },
    })

    const removeItemSpy = vi.spyOn(window.localStorage, 'removeItem')
    renderEditor()

    await waitFor(() => {
      expect(currentNodePosition('a')).toEqual({ x: 80, y: 90 })
      expect(currentNodePosition('b')).toEqual({ x: 520, y: 120 })
    })

    queueElkLayout({
      a: { x: 20, y: 30 },
      b: { x: 360, y: 210 },
    })
    await user.click(screen.getByRole('button', { name: 'Arrange' }))

    await waitFor(() => {
      expect(currentNodePosition('a')).toEqual({ x: 20, y: 30 })
      expect(currentNodePosition('b')).toEqual({ x: 360, y: 210 })
      const persistedLayout = readSavedLayout('flow-a.dot')
      expect(persistedLayout.nodePositions).toMatchObject({
        a: { x: 20, y: 30 },
        b: { x: 360, y: 210 },
      })
    })

    queueElkLayout({
      a: { x: 140, y: 10 },
      b: { x: 500, y: 10 },
    })
    await user.click(screen.getByRole('button', { name: 'Reset' }))

    await waitFor(() => {
      expect(removeItemSpy).toHaveBeenCalledWith(storageKeyFor('flow-a.dot'))
      expect(currentNodePosition('a')).toEqual({ x: 140, y: 10 })
      expect(currentNodePosition('b')).toEqual({ x: 500, y: 10 })
      const persistedLayout = readSavedLayout('flow-a.dot')
      expect(persistedLayout.nodePositions).toMatchObject({
        a: { x: 140, y: 10 },
        b: { x: 500, y: 10 },
      })
    })
  })

  it('does not overwrite the editable parent-only sidecar layout when expanded child preview mode is toggled on and back off', async () => {
    const user = userEvent.setup()
    const parentLayout = buildSavedLayout(
      {
        start: { x: 40, y: 20 },
        manager: { x: 420, y: 20 },
      },
      [{ from: 'start', to: 'manager' }],
    )
    window.localStorage.setItem(storageKeyFor('flow-a.dot'), JSON.stringify(parentLayout))
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({
      name: flowName,
      content: flowName,
    }))
    vi.mocked(loadEditorPreview).mockImplementation(async (_dot: string, _init, options) => {
      const flowName = options?.flowName ?? 'unknown.dot'
      if (flowName !== 'flow-a.dot') {
        throw new Error(`Unexpected preview request for ${flowName}`)
      }
      return buildManagerPreview(options?.expandChildren === true)
    })
    queueElkLayout({
      start: { x: 0, y: 0 },
      manager: { x: 320, y: 0 },
    })

    renderEditor()

    await waitFor(() => {
      expect(readSavedLayout('flow-a.dot')).toEqual(parentLayout)
      expect(screen.getByRole('button', { name: 'Expanded' })).toBeInTheDocument()
    })

    await user.click(screen.getByRole('button', { name: 'Expanded' }))

    await waitFor(() => {
      expect(screen.getByText('Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.')).toBeInTheDocument()
      expect(readSavedLayout('flow-a.dot')).toEqual(parentLayout)
    })

    await user.click(screen.getByRole('button', { name: 'Parent' }))

    await waitFor(() => {
      expect(screen.queryByText('Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.')).not.toBeInTheDocument()
      expect(readSavedLayout('flow-a.dot')).toEqual(parentLayout)
    })
  })

  it('does not attempt DOT saves for position-only node movement changes', async () => {
    const preview = buildPreview(['a', 'b'], [{ from: 'a', to: 'b' }])
    usePreviewMap({ 'flow-a.dot': preview })
    queueElkLayout({
      a: { x: 0, y: 0 },
      b: { x: 320, y: 0 },
    })

    renderEditor()

    await waitFor(() => {
      expect(currentNodePosition('a')).toEqual({ x: 0, y: 0 })
      expect(currentNodePosition('b')).toEqual({ x: 320, y: 0 })
    })

    vi.mocked(flowPersistence.saveFlowContent).mockClear()
    vi.mocked(flowPersistence.saveFlowContentExpectingSemanticEquivalence).mockClear()
    vi.useFakeTimers()

    await act(async () => {
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 360, y: 0 },
          dragging: true,
        },
      ])
      reactFlowHarness.latestProps?.onNodesChange([
        {
          id: 'b',
          type: 'position',
          position: { x: 400, y: 40 },
          dragging: false,
        },
      ])
      vi.advanceTimersByTime(500)
    })

    expect(flowPersistence.saveFlowContent).not.toHaveBeenCalled()
    expect(flowPersistence.saveFlowContentExpectingSemanticEquivalence).not.toHaveBeenCalled()
  })
})
