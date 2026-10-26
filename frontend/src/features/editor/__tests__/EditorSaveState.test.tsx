import {
  primeFlowSaveBaseline,
  resetFlowSaveBaselines,
  retryLastSaveContent,
  saveFlowContent,
  waitForFlowSaves,
} from '@/lib/flowPersistence'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { useStore } from '@/store'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const DEMO_FLOW = { flow: { schema_version: '1.0', id: 'demo', nodes: {}, edges: [] } }

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
      message: 'Resolve highlighted validation errors, then save again.',
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
      message: 'Retry save now, and verify backend connectivity if this repeats.',
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
})
