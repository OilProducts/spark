import { Editor, viewportShowingNewStep } from '@/features/editor/Editor'
import { EditorGraphBridgeProvider } from '@/features/editor/EditorGraphBridgeContext'
import { Sidebar } from '@/features/editor/Sidebar'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { loadEditorFlowPayload, loadEditorPreview } from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

type SavedFlow = { nodes: Record<string, { label?: string; routes?: Record<string, { to: string }> }> }
type Save = { project_path: string; flow: SavedFlow }
type Graph = { nodes: Array<{ id: string; label: string; kind: string }>; edges: Array<{ from: string; to: string; label?: string }> }

const node = (id: string, label: string, kind = 'agent_task') => ({ id, label, kind })
// Held's incoming route is the second edge, so its canvas ID is e-work-held-1.
const PROJECT_A: Graph = {
  nodes: [node('start', 'Start', 'start'), node('work', 'Work'), node('held', 'Held', 'exit'), node('done', 'Done', 'exit')],
  edges: [{ from: 'start', to: 'work' }, { from: 'work', to: 'held', label: 'hold' }, { from: 'work', to: 'done', label: 'finish' }],
}

// A typed flow, whose routes carry their decision IDs and descriptions.
const REVIEW_FLOW = {
  schema_version: '2',
  id: 'flow_a',
  title: 'Flow A',
  inputs: [],
  nodes: {
    start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'step_1', description: 'Begin the review.' } } },
    step_1: {
      kind: 'agent_task',
      label: 'Review',
      config: { kind: 'agent_task', prompt: 'Review the change.' },
      routes: { approved: { to: 'done', description: 'The change is approved.' } },
    },
    done: { kind: 'exit', label: 'Done', config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } } },
  },
}
const previewOf = (flow: typeof REVIEW_FLOW) => async () => ({
  status: 'ok',
  graph: { nodes: Object.entries(flow.nodes).map(([id, entry]) => ({ id, label: entry.label, kind: entry.kind })), edges: [], graph_attrs: {} },
  flow,
  diagnostics: [],
  errors: [],
})

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
      edges: [...PROJECT_A.edges, { from: 'start', to: 'e-work-held-1', label: 'notify' }, { from: 'e-work-held-1', to: 'done' }],
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

  it('takes back Add step, then the deletion before it, and keeps the deleted step\'s ID meanwhile', async () => {
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    await deleteStep(user, 'step_1')
    await waitFor(() => expect(canvasNode('step_1')).toBeNull())

    await user.click(screen.getByRole('button', { name: 'Add step' }))
    expect(canvasNode('step_2')).toHaveTextContent('New step 2')
    // The toolbar's Undo says what it takes back, as Cmd/Ctrl+Z does it.
    await user.click(screen.getByRole('button', { name: 'Undo add step' }))
    await waitFor(() => expect(canvasNode('step_2')).toBeNull())
    expect(canvasNode('step_1')).toBeNull()
    expect(screen.getByTestId('editor-undo')).toHaveAccessibleName('Undo delete')
    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(canvasNode('step_1')).toHaveTextContent('Review'))
    expect(screen.getByTestId('editor-undo')).toBeDisabled()
    // The new step led nowhere, so no save carried it.
    expect(saves.some((save) => 'step_2' in save.flow.nodes)).toBe(false)
    await waitFor(() => expect(Object.keys(saves.at(-1)?.flow.nodes ?? {}).sort()).toEqual(['done', 'start', 'step_1']))
    const saved = saves.at(-1)!.flow
    expect(saved.nodes.step_1).toMatchObject({
      label: 'Review',
      config: { prompt: 'Review the change.' },
      routes: { approved: { to: 'done', description: 'The change is approved.' } },
    })
    expect(saved.nodes.start.routes).toEqual({ next: { to: 'step_1', description: 'Begin the review.' } })
  })

  it('adds a route from the step inspector as a drag on the canvas does, and opens it', async () => {
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    fireEvent.click(canvasNode('step_1') as HTMLElement)

    const routes = await screen.findByRole('region', { name: 'Routes' })
    expect(within(routes).getByRole('button', { name: 'approved to Done' })).toBeVisible()
    await user.selectOptions(within(routes).getByRole('combobox', { name: 'New route to' }), 'Review (this step)')
    await user.type(within(routes).getByRole('textbox', { name: 'Decision ID (optional)' }), 'revise')
    await user.click(within(routes).getByRole('button', { name: 'Add route' }))
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1.routes).toEqual({
      approved: { to: 'done', description: 'The change is approved.' },
      revise: { to: 'step_1', description: 'Continue to Review.' },
    }))

    await user.click(within(routes).getByRole('button', { name: 'revise to Review' }))
    expect(screen.getByRole('heading', { name: 'Route settings' })).toHaveFocus()
    expect(screen.getByRole('textbox', { name: 'Decision ID' })).toHaveValue('revise')
    // From and to the same step.
    await user.click(within(screen.getByTestId('edge-structured-form')).getAllByRole('button', { name: 'Review' })[0])
    expect(screen.getByRole('heading', { name: 'Step settings' })).toHaveFocus()
    expect(useStore.getState().selectedNodeId).toBe('step_1')
  })

  it('saves a new step without a model of its own, so it follows the flow default', async () => {
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(REVIEW_FLOW))
    useStore.setState({ uiDefaults: { ...useStore.getState().uiDefaults, llm_model: 'default-model', reasoning_effort: 'high' } })
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    await user.click(screen.getByRole('button', { name: 'Add step' }))
    const addRouteTo = async (target: string) => {
      const routes = await screen.findByRole('region', { name: 'Routes' })
      await user.selectOptions(within(routes).getByRole('combobox', { name: 'New route to' }), target)
      await user.click(within(routes).getByRole('button', { name: 'Add route' }))
    }
    await addRouteTo('Done')
    fireEvent.click(canvasNode('step_1') as HTMLElement)
    await addRouteTo('New step 2')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_2).toBeDefined())
    expect(saves.at(-1)!.flow.nodes.step_2).not.toHaveProperty('execution')
  })

  it('moves a route to another step and deletes it from the route inspector', async () => {
    const flow = {
      ...REVIEW_FLOW,
      nodes: {
        ...REVIEW_FLOW.nodes,
        step_1: { ...REVIEW_FLOW.nodes.step_1, routes: { approved: { to: 'done', description: 'Continue to Done.' }, held: { to: 'held', description: 'Held for a person.' } } },
        held: { kind: 'exit', label: 'Held', config: { kind: 'exit', result: { name: 'held', outcome: 'failure' } } },
      },
    }
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(flow as unknown as typeof REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    fireEvent.click(canvasNode('step_1') as HTMLElement)
    await user.click(within(await screen.findByRole('region', { name: 'Routes' })).getByRole('button', { name: 'approved to Done' }))

    // Moved where it stands, keeping its decision; the description it was given names the new step.
    await user.selectOptions(screen.getByRole('combobox', { name: 'Goes to' }), 'Held')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1.routes).toEqual({
      approved: { to: 'held', description: 'Continue to Held.' },
      held: { to: 'held', description: 'Held for a person.' },
    }))
    expect(Object.keys(saves.at(-1)!.flow.nodes.step_1.routes!)).toEqual(['approved', 'held'])

    // Deleting it returns to the step it left; Cmd/Ctrl+Z puts it back.
    await user.click(screen.getByRole('button', { name: 'Delete route' }))
    expect(screen.getByRole('heading', { name: 'Step settings' })).toHaveFocus()
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1.routes).toEqual({ held: { to: 'held', description: 'Held for a person.' } }))
    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(Object.keys(saves.at(-1)?.flow.nodes.step_1.routes ?? {}).sort()).toEqual(['approved', 'held']))
    // Again, and the move is taken back too.
    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1.routes?.approved).toEqual({ to: 'done', description: 'Continue to Done.' }))
  })

  it('does not undo a deletion into the same flow name in another project', async () => {
    graphs['/project-b'] = {
      nodes: [node('start', 'Start', 'start'), node('review', 'Review'), node('done', 'Done', 'exit')],
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

describe('the view after Add step', () => {
  const box = (x: number, y = 0) => ({ x, y, width: 200, height: 80 })
  const canvas = { width: 1000, height: 600 }
  const onScreen = (rect: ReturnType<typeof box>, view: { x: number; y: number; zoom: number }) => {
    const left = view.x + rect.x * view.zoom
    const top = view.y + rect.y * view.zoom
    return left >= 0 && top >= 0 && left + rect.width * view.zoom <= canvas.width && top + rect.height * view.zoom <= canvas.height
  }

  it('shows the whole flow with its new step when it fits, never zooming in', () => {
    const flow = [box(0), box(280), box(560, 120)]
    const step = box(840)
    const view = viewportShowingNewStep(step, flow, canvas, 1)
    expect(view.zoom).toBeLessThanOrEqual(1)
    expect([...flow, step].every((rect) => onScreen(rect, view))).toBe(true)
    expect(viewportShowingNewStep(box(280), [box(0)], canvas, 1).zoom).toBe(1)
  })

  it('keeps the new step in view beside the flow it can\'t fit, at a zoom that still reads', () => {
    const flow = Array.from({ length: 12 }, (_, index) => box(index * 280))
    const step = box(12 * 280)
    const view = viewportShowingNewStep(step, flow, canvas, 1)
    expect(view.zoom).toBe(0.6)
    expect(onScreen(step, view)).toBe(true)
    expect(onScreen(flow.at(-1)!, view)).toBe(true)
  })
})
