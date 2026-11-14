import { Editor } from '@/features/editor/Editor'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { loadEditorFlowPayload, loadEditorPreview } from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

type SavedFlow = { nodes: Record<string, { routes?: Record<string, { to: string }> }> }
type Save = { project_path: string; flow: SavedFlow }
type Graph = { nodes: Array<{ id: string; label: string; shape: string }>; edges: Array<{ from: string; to: string; label?: string }> }

const node = (id: string, label: string, shape = 'box') => ({ id, label, shape })
// Held's incoming route is the second edge, so its canvas ID is e-work-held-1.
const PROJECT_A: Graph = {
  nodes: [node('start', 'Start', 'Mdiamond'), node('work', 'Work'), node('held', 'Held', 'Msquare'), node('done', 'Done', 'Msquare')],
  edges: [{ from: 'start', to: 'work' }, { from: 'work', to: 'held', label: 'hold' }, { from: 'work', to: 'done', label: 'finish' }],
}

const canvasNode = (id: string) => document.querySelector(`.react-flow__node[data-id="${id}"]`)
const routesTo = (flow: SavedFlow, target: string) => Object.values(flow.nodes)
  .flatMap((entry) => Object.values(entry.routes ?? {}))
  .filter((route) => route.to === target)

describe('Editor step deletion', () => {
  let graphs: Record<string, Graph>
  let saves: Save[]

  beforeEach(() => {
    resetFlowSaveBaselines()
    graphs = { '/project-a': PROJECT_A }
    saves = []
    useStore.setState((state) => ({
      ...state,
      viewMode: 'editor',
      preferredEditorMode: 'structured',
      preferredExpandChildFlows: false,
      editorExpandChildFlowsByFlow: {},
      activeProjectPath: '/project-a',
      activeFlow: 'flow-a',
      executionFlow: null,
      suppressPreview: true,
      selectedNodeId: null,
      selectedEdgeId: null,
    }))
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({ name: flowName, files: [], flow: { id: flowName } }))
    // Each project has its own flow of the same name.
    vi.mocked(loadEditorPreview).mockImplementation(async () => ({
      status: 'ok',
      graph: { ...graphs[useStore.getState().activeProjectPath ?? ''], graph_attrs: {} },
      diagnostics: [],
      errors: [],
    }))
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (String(url).endsWith('/api/flows') && init?.method === 'POST') {
        saves.push(JSON.parse(String(init.body)))
        return new Response(JSON.stringify({ status: 'saved' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response('{}', { status: 404 })
    }))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  const deleteStep = async (user: ReturnType<typeof userEvent.setup>, id: string) => {
    fireEvent.click(canvasNode(id) as HTMLElement)
    await user.keyboard('{Backspace}')
  }

  it('saves a deleted step without its routes, and Cmd+Z puts both back', async () => {
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())

    await deleteStep(user, 'held')
    await waitFor(() => expect(saves).toHaveLength(1))
    expect(Object.keys(saves[0].flow.nodes)).not.toContain('held')
    expect(routesTo(saves[0].flow, 'held')).toEqual([])
    expect(routesTo(saves[0].flow, 'done')).toHaveLength(1)
    expect(canvasNode('held')).toBeNull()

    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(saves).toHaveLength(2))
    expect(Object.keys(saves[1].flow.nodes)).toContain('held')
    expect(routesTo(saves[1].flow, 'held')).toHaveLength(1)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())
  })

  it('keeps a step whose ID is spelled like a deleted route\'s', async () => {
    graphs['/project-a'] = {
      nodes: [...PROJECT_A.nodes, node('e-work-held-1', 'Notify')],
      edges: [...PROJECT_A.edges, { from: 'start', to: 'e-work-held-1', label: 'notify' }],
    }
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())

    await deleteStep(user, 'held')
    await waitFor(() => expect(saves).toHaveLength(1))
    expect(Object.keys(saves[0].flow.nodes).sort()).toEqual(['done', 'e-work-held-1', 'start', 'work'])
    expect(routesTo(saves[0].flow, 'e-work-held-1')).toHaveLength(1)
    expect(canvasNode('e-work-held-1')).not.toBeNull()
  })

  it('does not undo a deletion into the same flow name in another project', async () => {
    graphs['/project-b'] = {
      nodes: [node('start', 'Start', 'Mdiamond'), node('review', 'Review'), node('done', 'Done', 'Msquare')],
      edges: [{ from: 'start', to: 'review' }, { from: 'review', to: 'done', label: 'finish' }],
    }
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())
    await deleteStep(user, 'held')
    await waitFor(() => expect(saves).toHaveLength(1))
    expect(saves[0].project_path).toBe('/project-a')

    // As a run's Open in editor does: its project, the same flow name.
    act(() => {
      useStore.getState().setActiveProjectPath('/project-b')
      useStore.getState().setActiveFlow('flow-a')
    })
    await waitFor(() => expect(canvasNode('review')).not.toBeNull())
    await user.keyboard('{Meta>}z{/Meta}')
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 400)) })
    expect(canvasNode('held')).toBeNull()
    expect(saves).toHaveLength(1)
  })
})
