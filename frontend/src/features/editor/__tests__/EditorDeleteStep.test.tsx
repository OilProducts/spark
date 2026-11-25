import { RemoteEnvironmentGate } from '@/app/EnvironmentSwitcher'
import { DialogProvider } from '@/components/app/dialog-controller'
import { Editor, viewportShowingNewStep } from '@/features/editor/Editor'
import { EditorGraphBridgeProvider } from '@/features/editor/EditorGraphBridgeContext'
import { GraphSettings } from '@/features/editor/GraphSettings'
import { Sidebar } from '@/features/editor/Sidebar'
import {
  EXPECTED_PROTOCOL_VERSION,
  EXPECTED_UI_API_VERSION,
  getActiveEnvironmentId,
  isWorkspaceBlocked,
  resetActiveEnvironmentForTests,
  reverifyActiveEnvironment,
  selectEnvironment,
  useActiveEnvironment,
} from '@/lib/activeEnvironment'
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
    resetActiveEnvironmentForTests()
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

  it('leaves the editor behind a failed switch\'s error gate untouched by keys pressed in the gate, and Return shows it as it was', async () => {
    const user = userEvent.setup()
    // As the app shows it over the hidden workspace.
    const Gate = () => (isWorkspaceBlocked(useActiveEnvironment()) ? <RemoteEnvironmentGate /> : null)
    render(<ReactFlowProvider><Editor /><Gate /></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())
    await deleteStep(user, 'held')
    await waitFor(() => expect(saves).toHaveLength(1))
    fireEvent.click(canvasNode('work') as HTMLElement)

    // The destination's check fails: its environment answers nothing it can verify.
    await act(async () => { await selectEnvironment('env-r') })
    const gate = screen.getByRole('dialog')
    expect(gate).toContainElement(screen.getByTestId('remote-environment-gate'))
    expect(gate).toContainElement(document.activeElement as HTMLElement)
    await user.keyboard('{Meta>}z{/Meta}{Backspace}')
    await act(() => new Promise((resolve) => setTimeout(resolve, 300)))
    expect(canvasNode('held')).toBeNull()
    expect(canvasNode('work')).not.toBeNull()
    expect(saves).toHaveLength(1)

    await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(saves).toHaveLength(2))
    expect(Object.keys(saves[1].flow.nodes)).toEqual(expect.arrayContaining(['held', 'work']))
  })

  it('keeps keys pressed in the confirmation of a departure from a revoked remote\'s gate off its editor, which Retry shows with its refused edit', async () => {
    const user = userEvent.setup()
    const Gate = () => (isWorkspaceBlocked(useActiveEnvironment()) ? <RemoteEnvironmentGate /> : null)
    const remote = '/workspace/api/environments/links/env-r/proxy'
    const environment = (id: string) => new Response(JSON.stringify({
      environment_id: id, display_name: id, protocol_version: EXPECTED_PROTOCOL_VERSION, ui_api_version: EXPECTED_UI_API_VERSION, spark_version: '',
    }), { headers: { 'Content-Type': 'application/json' } })
    let revoked = false
    const backend = vi.mocked(fetch).getMockImplementation()!
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      const url = String(input)
      if (revoked && url.startsWith(`${remote}/`)) return new Response(JSON.stringify({ detail: 'Unauthorized' }), { status: 401 })
      if (url === '/environment') return environment('local')
      if (url === `${remote}/workspace/api/environment`) return environment('env-r')
      return backend(input, init)
    })
    await act(async () => { expect(await selectEnvironment('env-r')).toBe(true) })
    render(<DialogProvider><ReactFlowProvider><Editor /><Gate /></ReactFlowProvider></DialogProvider>)
    await waitFor(() => expect(canvasNode('held')).not.toBeNull())

    // The remote revokes this Spark's token: it refuses the deletion's save, then the reconnect's check.
    revoked = true
    await deleteStep(user, 'held')
    await waitFor(() => expect(useStore.getState().saveState).toBe('error'))
    fireEvent.click(canvasNode('work') as HTMLElement)
    await act(async () => { await reverifyActiveEnvironment() })
    const gate = screen.getByTestId('remote-environment-gate')
    await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
    const leave = await screen.findByRole('alertdialog', { name: 'Discard unsaved flow edits?' })
    expect(leave).toContainElement(document.activeElement as HTMLElement)
    await user.keyboard('{Control>}z{/Control}{Backspace}')
    expect(canvasNode('held')).toBeNull()
    expect(canvasNode('work')).not.toBeNull()
    await user.click(within(leave).getByRole('button', { name: 'Keep editing' }))

    revoked = false
    await user.click(within(screen.getByTestId('remote-environment-gate')).getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument())
    expect(getActiveEnvironmentId()).toBe('env-r')
    expect(canvasNode('held')).toBeNull()
    expect(canvasNode('work')).not.toBeNull()
    expect(useStore.getState().saveState).toBe('error')
    expect(saves).toEqual([])
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
    await user.click(screen.getByTestId('editor-undo'))
    await waitFor(() => expect(canvasNode('step_1')).toHaveTextContent('Review'))
    // Nothing left to take back, and focus stays on the button.
    expect(screen.getByTestId('editor-undo')).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByTestId('editor-undo')).toHaveFocus()
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

  it('takes back an inspector edit with Undo, typing in one field as one edit', async () => {
    const flow = {
      ...REVIEW_FLOW,
      nodes: {
        ...REVIEW_FLOW.nodes,
        step_1: { ...REVIEW_FLOW.nodes.step_1, kind: 'tool', config: { kind: 'tool', command: 'make test', exit_codes: { '0': 'approved' } } },
      },
    }
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(flow as unknown as typeof REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    fireEvent.click(canvasNode('step_1') as HTMLElement)

    await user.type(await screen.findByRole('textbox', { name: 'Label' }), ' Checks')
    const exitCodes = screen.getByRole('textbox', { name: 'Exit codes' })
    await user.clear(exitCodes)
    await user.type(exitCodes, '0 = approved\n1 = failed')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Review Checks' }))

    // Undo names the step it sets back, by its name now.
    await user.click(screen.getByRole('button', { name: 'Undo edit to Review Checks' }))
    expect(exitCodes).toHaveValue('0 = approved')
    await user.click(screen.getByRole('button', { name: 'Undo edit to Review Checks' }))
    expect(screen.getByRole('textbox', { name: 'Label' })).toHaveValue('Review')
    expect(screen.getByTestId('editor-undo')).toHaveAttribute('aria-disabled', 'true')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Review', config: { exit_codes: { '0': 'approved' } } }))
  })

  it('takes back a flow settings edit with Undo, after a step edit made before it', async () => {
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /><GraphSettings /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    fireEvent.click(canvasNode('step_1') as HTMLElement)
    await user.type(await screen.findByRole('textbox', { name: 'Label' }), ' Changes')

    const flowDescription = within(screen.getByTestId('graph-structured-form')).getByRole('textbox', { name: /^Description/ })
    await user.click(flowDescription)
    await user.paste('Review a change, then ship it.')
    await waitFor(() => expect(saves.at(-1)?.flow).toMatchObject({ description: 'Review a change, then ship it.' }))

    await user.click(screen.getByRole('button', { name: 'Undo flow settings edit' }))
    expect(flowDescription).toHaveValue('')
    // Said on screen too, as the step it goes back on may not be in view.
    expect(screen.getByTestId('editor-undone')).toHaveTextContent('Undid flow settings edit')
    await waitFor(() => expect(saves.at(-1)?.flow).not.toHaveProperty('description', 'Review a change, then ship it.'))
    expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Review Changes' })
    await user.click(screen.getByRole('button', { name: 'Undo edit to Review Changes' }))
    expect(screen.getByTestId('editor-undone')).toHaveTextContent('Undid edit to Review')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Review' }))
  })

  it('saves an inspector edit made in one event, such as a paste, and a route edit after it', async () => {
    vi.mocked(loadEditorPreview).mockImplementation(previewOf(REVIEW_FLOW))
    const user = userEvent.setup()
    render(<ReactFlowProvider><EditorGraphBridgeProvider><Editor /><Sidebar /></EditorGraphBridgeProvider></ReactFlowProvider>)
    await waitFor(() => expect(canvasNode('step_1')).not.toBeNull())
    fireEvent.click(canvasNode('step_1') as HTMLElement)

    const label = await screen.findByRole('textbox', { name: 'Label' })
    await user.clear(label)
    await user.paste('Code review')
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Code review' }))

    await user.click(within(screen.getByRole('region', { name: 'Routes' })).getByRole('button', { name: 'approved to Done' }))
    fireEvent.change(await screen.findByRole('textbox', { name: /Description/ }), { target: { value: 'Ship it.' } })
    await waitFor(() => expect(saves.at(-1)?.flow.nodes.step_1.routes?.approved).toEqual({ to: 'done', description: 'Ship it.' }))
    expect(saves.at(-1)?.flow.nodes.step_1).toMatchObject({ label: 'Code review' })
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
    // Its badges sit above it, below the save status in the canvas's top right corner.
    expect(view.y + step.y * view.zoom).toBeGreaterThanOrEqual(80)
  })
})
