import { Editor } from '@/features/editor/Editor'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { fireEvent, render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { loadEditorFlowPayload, loadEditorPreview } from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

type SavedFlow = { nodes: Record<string, { routes?: Record<string, { to: string }> }> }

describe('Editor step deletion', () => {
  beforeEach(() => {
    resetFlowSaveBaselines()
    useStore.setState((state) => ({
      ...state,
      viewMode: 'editor',
      preferredEditorMode: 'structured',
      preferredExpandChildFlows: false,
      editorExpandChildFlowsByFlow: {},
      activeFlow: 'flow-a',
      executionFlow: null,
      suppressPreview: true,
      selectedNodeId: null,
      selectedEdgeId: null,
    }))
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({ name: flowName, files: [], flow: { id: flowName } }))
    vi.mocked(loadEditorPreview).mockResolvedValue({
      status: 'ok',
      graph: {
        nodes: [
          { id: 'start', label: 'Start', shape: 'Mdiamond' },
          { id: 'work', label: 'Work', shape: 'box' },
          { id: 'held', label: 'Held', shape: 'Msquare' },
          { id: 'done', label: 'Done', shape: 'Msquare' },
        ],
        edges: [
          { from: 'start', to: 'work' },
          { from: 'work', to: 'held', label: 'hold' },
          { from: 'work', to: 'done', label: 'finish' },
        ],
        graph_attrs: {},
      },
      diagnostics: [],
      errors: [],
    })
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('saves a deleted step without its routes, and Cmd+Z puts both back', async () => {
    const saved: SavedFlow[] = []
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (String(url).endsWith('/api/flows') && init?.method === 'POST') {
        saved.push(JSON.parse(String(init.body)).flow)
        return new Response(JSON.stringify({ status: 'saved' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response('{}', { status: 404 })
    }))
    const routesTo = (flow: SavedFlow, target: string) => Object.values(flow.nodes)
      .flatMap((node) => Object.values(node.routes ?? {}))
      .filter((route) => route.to === target)
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(document.querySelector('.react-flow__node[data-id="held"]')).not.toBeNull())

    fireEvent.click(document.querySelector('.react-flow__node[data-id="held"]') as HTMLElement)
    await user.keyboard('{Backspace}')
    await waitFor(() => expect(saved).toHaveLength(1))
    expect(Object.keys(saved[0].nodes)).not.toContain('held')
    expect(routesTo(saved[0], 'held')).toEqual([])
    expect(routesTo(saved[0], 'done')).toHaveLength(1)
    expect(document.querySelector('.react-flow__node[data-id="held"]')).toBeNull()

    await user.keyboard('{Meta>}z{/Meta}')
    await waitFor(() => expect(saved).toHaveLength(2))
    expect(Object.keys(saved[1].nodes)).toContain('held')
    expect(routesTo(saved[1], 'held')).toHaveLength(1)
    await waitFor(() => expect(document.querySelector('.react-flow__node[data-id="held"]')).not.toBeNull())
  })
})
