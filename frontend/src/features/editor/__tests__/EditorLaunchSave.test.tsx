import { Editor } from '@/features/editor/Editor'
import { resetFlowSaveBaselines } from '@/lib/flowPersistence'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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

vi.mock('@/features/editor/components/EditorCanvasToolbar', () => ({
  EditorCanvasToolbar: ({ onAddNode, onRun }: { onAddNode: () => void; onRun: (path: string) => void }) => (
    <div>
      <button type="button" onClick={onAddNode}>add node</button>
      <button type="button" onClick={() => onRun('/tmp/project')}>run</button>
    </div>
  ),
}))

const launched = vi.fn()
const launchErrors: string[] = []

vi.mock('@/features/launch', () => ({
  LaunchPanel: ({ target }: { target: { prepareLaunch?: () => Promise<void> } }) => (
    <button
      type="button"
      onClick={async () => {
        try {
          await target.prepareLaunch?.()
          launched()
        } catch (error) {
          launchErrors.push(String(error))
        }
      }}
    >
      launch
    </button>
  ),
}))

const saveResponse = (ok: boolean) => new Response(
  JSON.stringify(ok ? { status: 'saved' } : { detail: 'disk full' }),
  { status: ok ? 200 : 500, headers: { 'Content-Type': 'application/json' } },
)

describe('Editor launch preparation', () => {
  beforeEach(() => {
    resetFlowSaveBaselines()
    launched.mockReset()
    launchErrors.length = 0
    useStore.setState((state) => ({
      ...state,
      viewMode: 'editor',
      preferredEditorMode: 'structured',
      preferredExpandChildFlows: false,
      editorExpandChildFlowsByFlow: {},
      activeFlow: 'flow-a',
      executionFlow: null,
      suppressPreview: true,
    }))
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({ name: flowName, content: flowName }))
    vi.mocked(loadEditorPreview).mockResolvedValue({
      status: 'ok',
      graph: { nodes: [{ id: 'start', label: 'Start', shape: 'Mdiamond' }], edges: [], graph_attrs: {} },
      diagnostics: [],
      errors: [],
    })
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('does not launch after a failed autosave until the current content saves', async () => {
    const saveResults = [false, false, true]
    const savedContents: string[] = []
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (String(url).endsWith('/api/flows') && init?.method === 'POST') {
        savedContents.push(JSON.parse(String(init.body)).content)
        return saveResponse(saveResults.shift() ?? false)
      }
      return new Response('{}', { status: 404 })
    }))
    const user = userEvent.setup()
    render(<ReactFlowProvider><Editor /></ReactFlowProvider>)
    await waitFor(() => expect(useStore.getState().editorMode).toBe('structured'))
    await waitFor(() => expect(document.querySelector('[data-nodeid="start"]')).not.toBeNull())

    // The edit's autosave fails, and nothing is left pending.
    await user.click(screen.getByRole('button', { name: 'add node' }))
    await waitFor(() => expect(useStore.getState().saveState).toBe('error'))
    expect(savedContents).toHaveLength(1)

    await user.click(screen.getByRole('button', { name: 'run' }))
    const launch = await screen.findByRole('button', { name: 'launch' })

    // Run retries the current content; a second failure still blocks the launch.
    await user.click(launch)
    await waitFor(() => expect(launchErrors).toHaveLength(1))
    expect(launched).not.toHaveBeenCalled()
    expect(savedContents).toHaveLength(2)
    expect(savedContents[1]).toBe(savedContents[0])

    await user.click(launch)
    await waitFor(() => expect(launched).toHaveBeenCalledOnce())
    expect(savedContents).toHaveLength(3)
    expect(savedContents[2]).toBe(savedContents[0])
  })
})
