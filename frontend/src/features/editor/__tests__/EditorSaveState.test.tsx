import {
  primeFlowSaveBaseline,
  resetFlowSaveBaselines,
  retryLastSaveContent,
  saveFlowContent,
  waitForFlowSaves,
} from '@/lib/flowPersistence'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { useStore } from '@/store'
import { act, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EditorSaveStatus } from '../components/EditorSaveStatus'

const DEMO_FLOW = { flow: { schema_version: '2', id: 'demo', nodes: {} } }

const resetSaveState = () => {
  useStore.setState((state) => ({
    ...state,
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
  }))
}

describe('Editor save state behavior', () => {
  beforeEach(() => {
    resetSaveState()
    resetFlowSaveBaselines()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('refuses to save a node that declares one decision twice, without a request', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    const duplicated = {
      flow: {
        schema_version: '2',
        id: 'demo',
        nodes: {
          review: {
            kind: 'agent_task',
            routes: {
              ready: { to: 'done', description: 'Ready.' },
              'ready (duplicate 2)': { to: 'fix', description: 'Also ready.' },
            },
          },
        },
      },
    }
    expect(await saveFlowContent('demo', duplicated)).toBe(false)
    expect(fetchMock).not.toHaveBeenCalled()
    const state = useStore.getState()
    expect(state.saveErrorKind).toBe('validation_error')
    expect(state.saveErrorMessage).toContain('review.ready')
  })

  it('classifies parse and validation errors with actionable remediation', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(
          new Response(
            JSON.stringify({
              detail: { status: 'parse_error', error: 'line 3: expected node identifier' },
            }),
            { status: 400, headers: { 'Content-Type': 'application/json' } },
          ),
        )
        .mockResolvedValueOnce(
          new Response(
            JSON.stringify({
              detail: { status: 'validation_error', error: 'Unknown retry target node.' },
            }),
            { status: 422, headers: { 'Content-Type': 'application/json' } },
          ),
        ),
    )

    const parseSave = await saveFlowContent('demo', { files: [{ name: 'flow.md', content: '---\nid: [\n---\n' }] })
    expect(parseSave).toBe(false)
    let saveState = useStore.getState()
    expect(saveState.saveState).toBe('error')
    expect(saveState.saveErrorKind).toBe('parse_error')
    expect(saveState.saveErrorMessage).toContain('Save blocked by a flow parse error')
    expect(resolveSaveRemediation(saveState.saveState, saveState.saveErrorKind)).toEqual({
      message: 'Fix the flow file syntax in the Files view, then save again.',
      allowRetry: false,
    })

    const validationSave = await saveFlowContent('demo', DEMO_FLOW)
    expect(validationSave).toBe(false)
    saveState = useStore.getState()
    expect(saveState.saveState).toBe('error')
    expect(saveState.saveErrorKind).toBe('validation_error')
    expect(saveState.saveErrorMessage).toContain('Save blocked by validation errors')
    expect(resolveSaveRemediation(saveState.saveState, saveState.saveErrorKind)).toEqual({
      message: 'Fix the errors listed under Validation; the flow saves with your next change.',
      allowRetry: false,
    })
  })

  it('waits for in-flight saves to settle', async () => {
    let respond!: (response: Response) => void
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => { respond = resolve })))
    void saveFlowContent('demo', DEMO_FLOW)
    let settled = false
    const waited = waitForFlowSaves().then(() => { settled = true })
    await Promise.resolve()
    expect(settled).toBe(false)
    respond(new Response(JSON.stringify({ detail: 'disk full' }), { status: 500, headers: { 'Content-Type': 'application/json' } }))
    await waited
    expect(settled).toBe(true)
  })

  it('sends a flow’s next save only after the one in flight settles, so the newer content lands last', async () => {
    const responders: Array<(response: Response) => void> = []
    const fetchMock = vi.fn(() => new Promise<Response>((resolve) => { responders.push(resolve) }))
    vi.stubGlobal('fetch', fetchMock)
    const saved = () => new Response(JSON.stringify({ status: 'saved' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
    const older = saveFlowContent('demo', { flow: { ...DEMO_FLOW.flow, title: 'Older' } })
    const newer = saveFlowContent('demo', { flow: { ...DEMO_FLOW.flow, title: 'Newer' } })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(fetchMock).toHaveBeenCalledTimes(1)
    responders[0](saved())
    expect(await older).toBe(true)
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2))
    const bodies = fetchMock.mock.calls.map((call) => JSON.parse(String((call as unknown as [string, RequestInit])[1].body)).flow.title)
    expect(bodies).toEqual(['Older', 'Newer'])
    responders[1](saved())
    expect(await newer).toBe(true)
    expect(useStore.getState().saveState).toBe('saved')
  })

  it('supports retrying a transient network failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockRejectedValueOnce(new Error('offline'))
        .mockResolvedValueOnce(
          new Response(JSON.stringify({ status: 'saved' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          }),
        ),
    )

    const firstAttempt = await saveFlowContent('demo', DEMO_FLOW)
    expect(firstAttempt).toBe(false)
    let saveState = useStore.getState()
    expect(saveState.saveState).toBe('error')
    expect(saveState.saveErrorKind).toBe('network')
    expect(resolveSaveRemediation(saveState.saveState, saveState.saveErrorKind)).toEqual({
      message: 'Retry the save. If it fails again, check that Spark is still running.',
      allowRetry: true,
    })

    const retryAttempt = await retryLastSaveContent()
    expect(retryAttempt).toBe(true)
    saveState = useStore.getState()
    expect(saveState.saveState).toBe('saved')
    expect(saveState.saveErrorMessage).toBeNull()
    expect(saveState.saveErrorKind).toBeNull()
  })

  it('sends the flow as JSON with the active project', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ status: 'saved' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)
    useStore.setState({ activeProjectPath: '/tmp/project-a' })

    const saved = await saveFlowContent('demo', DEMO_FLOW)
    expect(saved).toBe(true)

    const [, requestInit] = fetchMock.mock.calls[0]
    const body = JSON.parse(String((requestInit as RequestInit).body))
    expect(body).toEqual({
      name: 'demo',
      project_path: '/tmp/project-a',
      ...DEMO_FLOW,
    })
  })

  it('skips no-op saves that match the loaded baseline', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)

    primeFlowSaveBaseline('demo', DEMO_FLOW)

    const saved = await saveFlowContent('demo', DEMO_FLOW)
    expect(saved).toBe(true)
    expect(fetchMock).not.toHaveBeenCalled()

    const saveState = useStore.getState()
    expect(saveState.saveState).toBe('idle')
    expect(saveState.saveErrorMessage).toBeNull()
    expect(saveState.saveErrorKind).toBeNull()
  })

  it('shows the save state in the toolbar: a brief Saved, and why edits are not saved', () => {
    vi.useFakeTimers()
    try {
      const reload = vi.fn()
      render(<EditorSaveStatus onReload={reload} />)
      const status = screen.getByTestId('editor-save-status')
      act(() => useStore.getState().markSaveSuccess())
      expect(status).toHaveTextContent('Saved')
      act(() => { vi.advanceTimersByTime(2000) })
      expect(status).toHaveTextContent('')
      expect(useStore.getState().saveState).toBe('idle')

      act(() => useStore.getState().markSaveFailure('Save blocked by validation errors: x', 'validation_error'))
      expect(status).toHaveTextContent(/^Not saved: Fix the errors listed under Validation/)
      expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()

      act(() => useStore.getState().markSaveFailure('Flow save failed: offline', 'network'))
      expect(status).toHaveTextContent('Save failed: Flow save failed: offline')
      expect(screen.getByText(/^Flow save failed: offline/)).toBeVisible()
      expect(screen.getByTestId('editor-save-retry')).toBeVisible()
      expect(screen.queryByTestId('editor-save-reload')).not.toBeInTheDocument()

      // A conflict says how to get out of it, with a button to do it, not only in a tooltip.
      act(() => useStore.getState().markSaveConflict('The flow was changed elsewhere since it was opened.'))
      expect(screen.getByText(/Reload the flow to get the saved version/)).toBeVisible()
      expect(screen.queryByTestId('editor-save-retry')).not.toBeInTheDocument()
      act(() => screen.getByTestId('editor-save-reload').click())
      expect(reload).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })
})
