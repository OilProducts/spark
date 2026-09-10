import { act, render, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { RunStream } from '../RunStream'
import { WorkspaceLiveEventsController } from '@/app/AppSessionControllers'
import { useRunJournalStore } from '../state/runJournalStore'
import { useRunTranscriptStore } from '../state/runTranscriptStore'
import { useRunsList } from '../hooks/useRunsList'
import { selectSelectedRunSession } from '@/state/runsSessionSelectors'
import { buildRunNodeStatuses } from '../model/nodeStatusModel'
import { ApiHttpError, loadSelectedRunStatus, loadSelectedRunJournal, loadRunTranscript } from '../services/runStreamTransport'
import type { PipelineStatusResponse } from '@/lib/attractorClient'

vi.mock('../services/runStreamTransport', async (original) => ({
    ...await original<typeof import('../services/runStreamTransport')>(),
    loadSelectedRunStatus: vi.fn(),
    loadSelectedRunJournal: vi.fn(),
    loadRunTranscript: vi.fn(),
}))

const pending: { runId: string; resolve: (value: PipelineStatusResponse) => void; reject: (error: Error) => void }[] = []
const select = (runId: string) => act(() => useStore.getState().setRunsSelectedRunIdForScope('all', runId))
const snapshot = (runId: string, status = 'running') => ({
    run_id: runId, pipeline_id: runId, status, flow_name: 'flow', project_path: '/a',
    working_directory: '/a', model: '', started_at: '', last_error: '', completed_nodes: ['done'],
}) as PipelineStatusResponse
const dispatch = (name: string, detail: unknown) => act(() => {
    window.dispatchEvent(new CustomEvent(name, { detail }))
})

beforeEach(() => {
    pending.length = 0
    useRunJournalStore.setState(useRunJournalStore.getInitialState(), true)
    useRunTranscriptStore.setState(useRunTranscriptStore.getInitialState(), true)
    useStore.setState(useStore.getInitialState(), true)
    useStore.getState().updateRunsListSession({ scopeMode: 'all' })
    vi.mocked(loadSelectedRunStatus).mockImplementation((runId) => new Promise((resolve, reject) => pending.push({ runId, resolve, reject })))
    vi.mocked(loadSelectedRunJournal).mockResolvedValue({ pipeline_id: 'a', entries: [], oldest_sequence: null, newest_sequence: null, has_older: false })
    vi.mocked(loadRunTranscript).mockResolvedValue({ run_id: 'a', segments: [], newest_sequence: null })
})
afterEach(() => {
    vi.useRealTimers()
    vi.clearAllMocks()
    vi.unstubAllGlobals()
})

it('ignores an old 404 and status response through A → B → A without restarting on record updates', async () => {
    select('a')
    render(<RunStream />)
    select('b')
    await act(async () => pending[0].reject(new ApiHttpError('/status', 404, 'missing')))
    expect(useStore.getState().runsListSession.selectedRunIdByScopeKey.all).toBe('b')
    select('a')
    await act(async () => pending[2].resolve(snapshot('a', 'completed')))
    await act(async () => pending[1].resolve(snapshot('b')))
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('completed')
    expect(useStore.getState().runDetailSessionsByRunId.b.record).toBeNull()
    act(() => useStore.getState().reconcileRunRecord('a', 'journal', { token_usage: 20 }))
    expect(pending).toHaveLength(3)
})

it('does not let removed session callbacks write into a recreated session', async () => {
    select('a')
    render(<RunStream />)
    act(() => useStore.getState().clearRunDetailSession('a'))
    select('a')
    await act(async () => pending[0].resolve(snapshot('a', 'failed')))
    expect(useStore.getState().runDetailSessionsByRunId.a.record).toBeNull()
    await act(async () => pending[1].resolve(snapshot('a')))
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('running')
})

it('coalesces gaps and still refreshes terminal durable data when another listener already patched the record', async () => {
    select('a')
    render(<RunStream />)
    await act(async () => pending[0].resolve(snapshot('a')))
    act(() => useStore.getState().updateRunDetailSession('a', { nodeStatuses: { stale: 'running' } }))
    for (let i = 0; i < 5; i++) dispatch('spark:run-resync-required', { runId: 'a' })
    expect(pending).toHaveLength(2)
    expect(useStore.getState().runDetailSessionsByRunId.a.nodeStatuses).toEqual({})
    act(() => useStore.getState().reconcileRunRecord('a', 'live', { status: 'completed' }))
    dispatch('spark:run-upsert', { run: { run_id: 'a', status: 'completed' } })
    await act(async () => pending[1].resolve(snapshot('a')))
    expect(pending).toHaveLength(3)
    await act(async () => pending[2].resolve(snapshot('a', 'completed')))
    dispatch('spark:run-upsert', { run: { run_id: 'a', status: 'completed' } })
    expect(pending).toHaveLength(3)
    expect(useStore.getState().runDetailSessionsByRunId.a.completedNodesSnapshot).toEqual(['done'])
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('completed')
})

it.each(['runtime', 'upsert'])('re-arms terminal refresh after a same-selection retry via %s, including recovery', async (source) => {
    select('a')
    render(<RunStream />)
    await act(async () => pending[0].resolve(snapshot('a')))
    const emit = (status: string) => {
        // Simulate the list listener winning the listener-order race.
        act(() => useStore.getState().reconcileRunRecord('a', 'live', { status }))
        if (source === 'runtime') {
            dispatch('spark:run-journal-entry', { runId: 'a', entry: { type: 'runtime', status } })
        } else {
            dispatch('spark:run-upsert', { run: { run_id: 'a', status } })
        }
    }
    emit('failed')
    emit('failed')
    expect(pending).toHaveLength(2)
    await act(async () => pending[1].resolve({ ...snapshot('a', 'failed'), completed_nodes: ['failed-task'] }))
    expect(useStore.getState().runDetailSessionsByRunId.a.completedNodesSnapshot).toEqual(['failed-task'])
    emit('running')
    dispatch('spark:run-resync-required', { runId: 'a' })
    expect(pending).toHaveLength(3)
    emit('completed')
    emit('completed')
    await act(async () => pending[2].resolve(snapshot('a')))
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('completed')
    expect(pending).toHaveLength(4)
    await act(async () => pending[3].resolve({ ...snapshot('a', 'completed'), completed_nodes: ['retry-task'] }))
    emit('completed')
    expect(pending).toHaveLength(4)
    expect(useStore.getState().runDetailSessionsByRunId.a.completedNodesSnapshot).toEqual(['retry-task'])
})

it('reconciles obsolete overlays on revisit while keeping cached rendering through failure and newer live events', async () => {
    const transport = await vi.importActual<typeof import('../services/runStreamTransport')>('../services/runStreamTransport')
    vi.mocked(loadSelectedRunStatus).mockImplementation(transport.loadSelectedRunStatus)
    vi.stubGlobal('fetch', vi.fn((input: string) => new Promise<Response>((resolve, reject) => {
        const runId = input.split('/').at(-1)!
        pending.push({ runId, reject, resolve: (value) => resolve(new Response(JSON.stringify(value), {
            status: 200, headers: { 'Content-Type': 'application/json' },
        })) })
    })))
    select('a')
    render(<RunStream />)
    await act(async () => pending[0].resolve(snapshot('a')))
    act(() => useStore.getState().updateRunDetailSession('a', {
        nodeStatuses: { task: 'waiting', other: 'running' },
        humanGate: { id: 'old', runId: 'a', nodeId: 'task', prompt: 'old question', options: [] },
    }))
    select('b')
    select('a')
    expect(useStore.getState().runDetailSessionsByRunId.a.humanGate?.id).toBe('old')
    await act(async () => pending[2].reject(new Error('offline')))
    expect(useStore.getState().runDetailSessionsByRunId.a.nodeStatuses.task).toBe('waiting')
    select('b')
    select('a')
    await act(async () => pending[4].resolve({ ...snapshot('a', 'completed'), completed_nodes: ['task', 'other'] }))
    const session = useStore.getState().runDetailSessionsByRunId.a
    expect(session.humanGate).toBeNull()
    expect(buildRunNodeStatuses({ completedNodes: session.completedNodesSnapshot, nodeOutcomes: {}, currentNodeId: null,
        liveNodeStatuses: session.nodeStatuses, gateNodeId: session.humanGate?.nodeId ?? null,
        isRunActive: false, runStatus: session.record?.status ?? null })).toEqual({ task: 'success', other: 'success' })
    select('b')
    select('a')
    dispatch('spark:run-journal-entry', { runId: 'a', entry: { type: 'human_gate', node_id: 'task', question_id: 'new', prompt: 'new question' } })
    await act(async () => pending[6].resolve(snapshot('a', 'completed')))
    expect(useStore.getState().runDetailSessionsByRunId.a.humanGate?.id).toBe('new')
    expect(useStore.getState().runDetailSessionsByRunId.a.nodeStatuses.task).toBe('waiting')
})

it('hydrates durable details through validated HTTP while preserving live fields during the request', async () => {
    const transport = await vi.importActual<typeof import('../services/runStreamTransport')>('../services/runStreamTransport')
    vi.mocked(loadSelectedRunStatus).mockImplementation(transport.loadSelectedRunStatus)
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve, reject) => {
        pending.push({ runId: 'a', reject, resolve: (value) => resolve(new Response(JSON.stringify(value), {
            status: 200, headers: { 'Content-Type': 'application/json' },
        })) })
    })))
    select('a')
    useStore.getState().reconcileRunRecord('a', 'list', {
        run_id: 'a', flow_name: 'summary', status: 'running', project_path: '/a',
        working_directory: '/a', model: '', started_at: '', last_error: '',
    })
    render(<RunStream />)
    dispatch('spark:run-journal-entry', { runId: 'a', entry: { type: 'StageStarted', node_id: 'new-task', index: 1 } })
    // The workspace listener can update telemetry without changing live status.
    act(() => useStore.getState().reconcileRunRecord('a', 'live', { token_usage: 99 }))
    await act(async () => pending[0].resolve({ ...snapshot('a'), current_node: 'old-task', token_usage: 10, git_commit: 'durable' }))
    expect(useStore.getState().runDetailSessionsByRunId.a).toMatchObject({
        completedNodesSnapshot: ['done'], statusFetchedAtMs: expect.any(Number), statusSync: 'ready',
        nodeStatuses: { 'new-task': 'running' },
        record: { current_node: 'new-task', token_usage: 99, git_commit: 'durable', flow_name: 'flow' },
    })
    act(() => useStore.getState().updateRunDetailSession('a', { contextSearchQuery: 'ordinary update' }))
    expect(pending).toHaveLength(1)
})


it.each([false, true])('hydrates authoritative status after an intervening validated list response (cached summary: %s)', async (cached) => {
    const transport = await vi.importActual<typeof import('../services/runStreamTransport')>('../services/runStreamTransport')
    vi.mocked(loadSelectedRunStatus).mockImplementation(transport.loadSelectedRunStatus)
    const requests: { url: string; resolve: (response: Response) => void }[] = []
    vi.stubGlobal('fetch', vi.fn((url: string) => new Promise<Response>((resolve) => requests.push({ url, resolve }))))
    const respond = async (url: string, body: unknown) => {
        const request = requests.find((request) => request.url.endsWith(url))
        expect(request).toBeDefined()
        await act(async () => request!.resolve(new Response(JSON.stringify(body), {
            status: 200, headers: { 'Content-Type': 'application/json' },
        })))
    }
    select('a')
    if (cached) useStore.getState().updateRunsListSession({ runs: [{ ...snapshot('a'), flow_name: 'cached summary' }] })
    expect(selectSelectedRunSession(useStore.getState())?.record?.flow_name ?? null).toBe(cached ? 'cached summary' : null)
    render(<RunStream />)
    const list = renderHook(() => useRunsList({ activeProjectPath: null, scopeMode: 'all', selectedRunId: 'a' }))
    await respond('/runs', { runs: [{ ...snapshot('a'), flow_name: 'refreshed summary', current_node: 'stale', last_error: 'summary error' }] })
    expect(selectSelectedRunSession(useStore.getState())?.record?.flow_name).toBe('refreshed summary')
    await respond('/pipelines/a', { ...snapshot('a', 'completed'), current_node: 'done', git_commit: 'durable', completed_nodes: ['start', 'done'] })
    const session = selectSelectedRunSession(useStore.getState())!
    expect(session).toMatchObject({
        statusSync: 'ready', statusError: null, statusFetchedAtMs: expect.any(Number),
        completedNodesSnapshot: ['start', 'done'],
        record: { status: 'completed', flow_name: 'flow', current_node: 'done', git_commit: 'durable', last_error: '' },
    })
    expect(buildRunNodeStatuses({ completedNodes: session.completedNodesSnapshot, nodeOutcomes: {}, currentNodeId: session.record?.current_node ?? null,
        liveNodeStatuses: session.nodeStatuses, gateNodeId: null, isRunActive: false, runStatus: session.record?.status ?? null })).toMatchObject({ start: 'success', done: 'success' })
    expect(list.result.current.scopedRuns[0]).toBe(session.record)
    expect(list.result.current.summary.running).toBe(0)
})

it.each(['live', 'journal'])('retains the latest %s write matching a summary during validated status hydration', async (source) => {
    const transport = await vi.importActual<typeof import('../services/runStreamTransport')>('../services/runStreamTransport')
    vi.mocked(loadSelectedRunStatus).mockImplementation(transport.loadSelectedRunStatus)
    const requests: { url: string; resolve: (response: Response) => void }[] = []
    vi.stubGlobal('fetch', vi.fn((url: string) => new Promise<Response>((resolve) => requests.push({ url, resolve }))))
    const respond = async (url: string, body: unknown) => {
        const index = requests.findIndex((request) => request.url.endsWith(url))
        expect(index).toBeGreaterThanOrEqual(0)
        const [request] = requests.splice(index, 1)
        await act(async () => request.resolve(new Response(JSON.stringify(body), {
            status: 200, headers: { 'Content-Type': 'application/json' },
        })))
    }
    select('a')
    useStore.getState().updateRunsListSession({ runs: [{ ...snapshot('a'), flow_name: 'cached summary' }] })
    render(<RunStream />)
    const list = renderHook(() => useRunsList({ activeProjectPath: null, scopeMode: 'all', selectedRunId: 'a' }))
    const emit = (status: string) => source === 'live'
        ? dispatch('spark:run-upsert', { run: snapshot('a', status) })
        : dispatch('spark:run-journal-entry', { runId: 'a', entry: { type: 'runtime', status } })
    emit('completed')
    await respond('/runs', { runs: [{ ...snapshot('a'), flow_name: 'stale summary', current_node: 'stale', last_error: 'stale error' }] })
    emit('running')
    await respond('/pipelines/a', { ...snapshot('a'), current_node: 'fresh', git_commit: 'durable', last_error: 'durable error' })
    // The terminal event starts a durable refresh, superseding the initial request.
    await respond('/pipelines/a', { ...snapshot('a'), current_node: 'fresh', git_commit: 'durable', last_error: 'durable error' })
    const session = selectSelectedRunSession(useStore.getState())!
    expect(session).toMatchObject({
        statusSync: 'ready', completedNodesSnapshot: ['done'],
        record: { status: 'running', flow_name: 'flow', current_node: 'fresh', git_commit: 'durable',
            last_error: source === 'live' ? '' : 'durable error' },
    })
    expect(list.result.current.scopedRuns[0]).toBe(session.record)
    expect(list.result.current.summary.running).toBe(1)
})


it.each(['selection revisit', 'session recreation', 'connection replacement'])(
    'rejects captured workspace callbacks after %s before dispatch or cursor updates', async (transition) => {
        class Source {
            static instances: Source[] = []
            onmessage: ((event: MessageEvent<string>) => void) | null = null
            onerror: (() => void) | null = null
            close = vi.fn()
            constructor(readonly url: string) { Source.instances.push(this) }
        }
        vi.stubGlobal('EventSource', Source)
        vi.useFakeTimers()
        select('a')
        render(<><WorkspaceLiveEventsController /><RunStream /></>)
        await act(async () => pending[0].resolve(snapshot('a')))
        const original = Source.instances.at(-1)!
        const obsoleteMessage = original.onmessage!
        const obsoleteError = original.onerror!
        if (transition === 'selection revisit') {
            select('b')
            select('a')
            await act(async () => pending.at(-1)!.resolve(snapshot('a')))
        } else if (transition === 'session recreation') {
            act(() => useStore.getState().clearRunDetailSession('a'))
            select('a')
            await act(async () => pending.at(-1)!.resolve(snapshot('a')))
        } else {
            act(() => original.onerror!())
            act(() => vi.advanceTimersByTime(500))
        }
        const current = Source.instances.at(-1)!
        expect(current).not.toBe(original)
        const sessions = useStore.getState().runDetailSessionsByRunId
        const journal = useRunJournalStore.getState().byRunId
        const transcript = useRunTranscriptStore.getState().byRunId
        const requests = [loadSelectedRunStatus, loadSelectedRunJournal, loadRunTranscript].map((load) => vi.mocked(load).mock.calls.length)
        const events = [
            { sequence: 91, emitted_at: '2026-01-01T00:00:00Z', type: 'runtime', status: 'failed' },
            { type: 'run.journal_entry', resource: { kind: 'run', id: 'a' }, cursor: { kind: 'run_sequence', value: 92 },
                payload: { sequence: 92, emitted_at: '2026-01-01T00:00:00Z', type: 'human_gate', node_id: 'stale', question_id: 'stale', prompt: 'obsolete' } },
            { type: 'resync_required', resource: { kind: 'run', id: 'a' }, payload: { reason: 'gap' } },
            { type: 'conversation.segment_upsert', resource: { kind: 'node_execution', id: 'stale' },
                payload: { run_id: 'a', node_id: 'stale', attempt: 1, record: { source_event_sequence: 93,
                    segment: { id: 'segment', turn_id: 'turn', order: 0, kind: 'assistant_message', role: 'assistant',
                        status: 'complete', timestamp: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', content: 'message' } } } },
        ]
        const emit = (callback: typeof obsoleteMessage, envelope: unknown) => act(() => callback(new MessageEvent('message', { data: JSON.stringify(envelope) })))
        const dispatched = vi.fn()
        window.addEventListener('spark:run-segment-upsert', dispatched)
        try {
            for (const event of events) emit(obsoleteMessage, event)
            expect(useStore.getState().runDetailSessionsByRunId).toBe(sessions)
            expect(useRunJournalStore.getState().byRunId).toBe(journal)
            expect(useRunTranscriptStore.getState().byRunId).toBe(transcript)
            expect(dispatched).not.toHaveBeenCalled()
            expect([loadSelectedRunStatus, loadSelectedRunJournal, loadRunTranscript].map((load) => vi.mocked(load).mock.calls.length)).toEqual(requests)
            act(() => obsoleteError())
            act(() => vi.advanceTimersByTime(500))
            expect(Source.instances.at(-1)).toBe(current)
            expect(current.close).not.toHaveBeenCalled()
            // Reopening reveals whether an obsolete envelope poisoned the transport cursor.
            act(() => current.onerror!())
            act(() => vi.advanceTimersByTime(500))
            const replacement = Source.instances.at(-1)!
            expect(new URL(replacement.url, window.location.href).searchParams.get('run_sequence')).toBe(
                new URL(current.url, window.location.href).searchParams.get('run_sequence'),
            )
            emit(replacement.onmessage!, { sequence: 1, emitted_at: '2026-01-01T00:00:00Z', type: 'human_gate', node_id: 'fresh', question_id: 'fresh', prompt: 'current' })
            expect(useStore.getState().runDetailSessionsByRunId.a).toMatchObject({
                record: { current_node: 'fresh' }, nodeStatuses: { fresh: 'waiting' }, humanGate: { id: 'fresh' },
            })
            expect(useRunJournalStore.getState().byRunId.a.newestSequence).toBe(1)
            emit(replacement.onmessage!, events[3])
            expect(useRunTranscriptStore.getState().byRunId.a.segments[0]).toMatchObject({ id: 'segment', content: 'message' })
            emit(replacement.onmessage!, events[2])
            expect(loadSelectedRunStatus).toHaveBeenCalledTimes(requests[0] + 1)
        } finally {
            window.removeEventListener('spark:run-segment-upsert', dispatched)
        }
    },
)

it('broadcasts detected journal gaps so question snapshots also reconcile', async () => {
    select('a')
    render(<RunStream />)
    await act(async () => pending[0].resolve(snapshot('a')))
    const resync = vi.fn()
    window.addEventListener('spark:run-resync-required', resync)
    try {
        for (const sequence of [1, 3]) {
            dispatch('spark:run-journal-entry', {
                runId: 'a', entry: { type: 'StageStarted', node_id: 'work', sequence, emitted_at: '2026-09-08T00:00:00Z' },
            })
        }
        expect(resync).toHaveBeenCalledTimes(1)
        expect(resync.mock.calls[0][0].detail).toEqual({ runId: 'a', reason: 'gap' })
        expect(pending).toHaveLength(2)
        await act(async () => pending[1].resolve(snapshot('a')))
    } finally {
        window.removeEventListener('spark:run-resync-required', resync)
    }
})


it.each([
    ['root', 'a', 'a', 'a', 'root'],
    ['selected child', 'a', 'a', 'parent', 'root'],
    ['parent viewing child', 'a', 'child', 'a', 'child'],
])('receives live execution activity for %s', async (_, selected, owner, presentation, scope) => {
    class Source {
        static instances: Source[] = []
        onmessage: ((event: MessageEvent) => void) | null = null
        onerror: (() => void) | null = null
        close = vi.fn()
        constructor(readonly url: string) { Source.instances.push(this) }
    }
    vi.stubGlobal('EventSource', Source)
    select(selected)
    render(<><WorkspaceLiveEventsController /><RunStream /></>)
    await act(async () => pending[0].resolve(snapshot(selected)))
    const dispatched = vi.fn()
    const conversation = vi.fn()
    window.addEventListener('spark:run-segment-upsert', dispatched)
    window.addEventListener('spark:conversation-live-event', conversation)
    const segment = {
        id: 'live-segment', turn_id: 'turn', order: 0, kind: 'assistant_message', role: 'assistant',
        status: 'complete', timestamp: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', content: 'Live progress',
    }
    const emit = (resource: object, payload: object) => act(() => Source.instances.at(-1)!.onmessage!(
        new MessageEvent('message', { data: JSON.stringify({ type: 'conversation.segment_upsert', resource, payload }) }),
    ))
    try {
        emit({ kind: 'node_execution', id: `${owner}:work:1:0` }, {
            run_id: owner, presentation_run_id: presentation, node_id: 'work', attempt: 0,
            source_scope: owner === presentation ? 'root' : 'child',
            record: { source_event_sequence: 7, segment },
        })
        expect(useRunTranscriptStore.getState().byRunId[selected].segments).toEqual([
            expect.objectContaining({ id: 'live-segment', content: 'Live progress', node_id: 'work',
                source_run_id: owner, source_scope: scope, latest_sequence: 7 }),
        ])
        expect(dispatched).toHaveBeenCalledTimes(owner === presentation ? 1 : 2)
        expect(conversation).not.toHaveBeenCalled()
        emit({ kind: 'conversation', id: 'chat' }, { segment })
        expect(conversation).toHaveBeenCalledTimes(1)
        expect(dispatched).toHaveBeenCalledTimes(owner === presentation ? 1 : 2)
    } finally {
        window.removeEventListener('spark:run-segment-upsert', dispatched)
        window.removeEventListener('spark:conversation-live-event', conversation)
    }
})
