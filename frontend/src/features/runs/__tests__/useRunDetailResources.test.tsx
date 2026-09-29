import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { useRunDetailResources } from '../hooks/useRunDetailResources'
import { useRunDetails } from '../hooks/useRunDetails'
import type { RunRecord } from '../model/shared'

import { useRunTimeline } from '../hooks/useRunTimeline'
import { useRunJournalStore } from '../state/runJournalStore'
import { toTimelineEvent } from '../model/timelineModel'

const pending: { resolve: (response: Response) => void }[] = []

beforeEach(() => {
    pending.length = 0
    useStore.setState(useStore.getInitialState(), true)
    useStore.getState().setRunsSelectedRunIdForScope('all', 'run')
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
        pending.push({ resolve })
    })))
})
afterEach(() => vi.unstubAllGlobals())

it.each([
    ['b.txt', 200],
    ['b.txt', 500],
    ['a.txt', 200],
    ['a.txt', 500],
])('keeps the latest preview of %s when an older request finishes with %s', async (path, status) => {
    const { result, unmount } = renderHook(() => useRunDetailResources({ selectedRunId: 'run', manageSync: false }))
    let first!: Promise<void>
    act(() => { first = result.current.viewArtifact({ path: 'a.txt', viewable: true }) })
    unmount()
    const latest = renderHook(() => useRunDetailResources({ selectedRunId: 'run', manageSync: false }))
    let second!: Promise<void>
    act(() => { second = latest.result.current.viewArtifact({ path, viewable: true }) })
    await act(async () => {
        pending[1].resolve(new Response('Latest contents'))
        await second
    })
    await act(async () => {
        pending[0].resolve(new Response('Old response', { status }))
        await first
    })
    expect(latest.result.current.selectedArtifactPath).toBe(path)
    expect(latest.result.current.artifactViewerPayload).toBe('Latest contents')
    expect(latest.result.current.artifactViewerStatus).toBe('ready')
    expect(latest.result.current.artifactViewerError).toBeNull()
})

it.each(['spark:runs-transport-reconnect', 'spark:run-resync-required'])(
    'reconciles dropped parent/child questions and answers on %s, and removes terminal orphans without losing history',
    async (signal) => {
        const question = (id: string, runId: string) => ({
            emitted_at: '2026-09-08T00:00:00Z', question_id: id, run_id: runId, origin: 'agent_clarification', node_id: 'work',
            prompt: `Question ${id}`, question_type: 'FREEFORM', options: [],
        })
        let questions = [] as ReturnType<typeof question>[]
        vi.stubGlobal('fetch', vi.fn((url: string) => url.endsWith('/questions')
            ? Promise.resolve(Response.json({ questions }))
            : new Promise<Response>(() => {})))
        useRunJournalStore.setState(useRunJournalStore.getInitialState(), true)
        const { result } = renderHook(() => {
            const resources = useRunDetailResources({ selectedRunId: 'run' })
            return useRunTimeline({
                pendingQuestionSnapshots: resources.pendingQuestionSnapshots,
                selectedRunTimelineId: 'run',
            })
        })
        const ids = () => result.current.groupedPendingInterviewGates.flatMap((group) => group.gates.map((gate) => [gate.questionId, gate.runId]))
        await waitFor(() => expect(useStore.getState().runDetailSessionsByRunId.run.questionsStatus).toBe('ready'))
        expect(ids()).toEqual([])
        // Both human_gate events were dropped. Only durable snapshots can restore controls.
        questions = [question('parent', 'run'), question('child', 'child')]
        await act(async () => window.dispatchEvent(new CustomEvent(signal, { detail: { runId: 'child' } })))
        expect(ids()).toEqual([['parent', 'run'], ['child', 'child']])
        // Durable journal reconciliation restores history independently of pending controls.
        const history = questions.map((q, i) => toTimelineEvent({ ...q, type: 'human_gate', sequence: i + 1 })!)
        history.unshift(toTimelineEvent({
            ...questions[0], type: 'InterviewCompleted', sequence: 3, answer: 'Custom answer',
        })!)
        act(() => useRunJournalStore.getState().mergeLatestPage('run', {
            entries: history, oldestSequence: 1, newestSequence: 3, hasOlder: false,
        }))
        // The answer event was missed live; the next snapshot excludes the accepted question.
        questions = [questions[1]]
        await act(async () => window.dispatchEvent(new CustomEvent(signal, { detail: { runId: 'run' } })))
        expect(ids()).toEqual([['child', 'child']])
        const restoredHistory = () => useRunJournalStore.getState().byRunId.run.segments
        const restoredHistoryBefore = restoredHistory()
        expect(JSON.stringify(restoredHistory())).toContain('Custom answer')
        expect(JSON.stringify(restoredHistory())).toContain('Question child')
        // No closing interview event arrives for a dead child request.
        questions = []
        await act(async () => window.dispatchEvent(new CustomEvent('spark:run-upsert', {
            detail: { run: { run_id: 'child', status: 'failed' } },
        })))
        expect(ids()).toEqual([])
        expect(result.current.confirmedQuestionIds).toEqual([])
        expect(restoredHistory()).toEqual(restoredHistoryBefore)
    },
)

it('refetches files when a stage settles and the result when the run ends', () => {
    const fetchMock = vi.mocked(global.fetch)
    renderHook(() => useRunDetailResources({ selectedRunId: 'run', manageSync: true }))
    const count = (resource: string) => fetchMock.mock.calls.filter(([url]) => String(url).endsWith(`/run/${resource}`)).length
    expect([count('artifacts'), count('result')]).toEqual([1, 1])
    const journal = (type: string, runId = 'run') => act(() => {
        window.dispatchEvent(new CustomEvent('spark:run-journal-entry', { detail: { runId, entry: { type } } }))
    })
    journal('StageStarted')
    journal('StageCompleted', 'other-run')
    expect([count('artifacts'), count('result')]).toEqual([1, 1])
    journal('StageCompleted')
    expect([count('artifacts'), count('result')]).toEqual([2, 1])
    journal('PipelineCompleted')
    expect([count('artifacts'), count('result')]).toEqual([3, 2])
})

// A backend double with the executor's ordering: PipelineCompleted is journaled
// before the result (and its files) are materialized.
function stubMaterializingBackend() {
    const backend = { status: 'running', materialized: false }
    vi.stubGlobal('fetch', vi.fn((url: string) => {
        if (url.endsWith('/run/result')) {
            const state = backend.status === 'running' ? 'pending' : backend.materialized ? 'ready' : 'unavailable'
            return Promise.resolve(Response.json({
                run_id: 'run', status: backend.status, state, body_markdown: backend.materialized ? 'Shipped it.' : '', summary_enabled: true,
            }))
        }
        if (url.endsWith('/run/artifacts')) {
            const artifacts = backend.materialized ? [{ path: 'result/summary.md', media_type: 'text/markdown' }] : []
            return Promise.resolve(Response.json({ pipeline_id: 'run', artifacts }))
        }
        return new Promise<Response>(() => {})
    }))
    return backend
}

it('shows the result and its files once they materialize after PipelineCompleted', async () => {
    vi.useFakeTimers()
    try {
        const backend = stubMaterializingBackend()
        const { result } = renderHook(() => useRunDetailResources({ selectedRunId: 'run' }))
        await act(() => vi.advanceTimersByTimeAsync(0))
        expect(result.current.resultData?.state).toBe('pending')
        backend.status = 'completed'
        act(() => {
            window.dispatchEvent(new CustomEvent('spark:run-journal-entry', { detail: { runId: 'run', entry: { type: 'PipelineCompleted' } } }))
        })
        await act(() => vi.advanceTimersByTimeAsync(0))
        expect(result.current.resultData?.state).toBe('unavailable')
        // The summary-model call finishes well after the final event.
        await act(() => vi.advanceTimersByTimeAsync(5000))
        backend.materialized = true
        await act(() => vi.advanceTimersByTimeAsync(60000))
        expect(result.current.resultData).toMatchObject({ state: 'ready', body_markdown: 'Shipped it.' })
        expect(result.current.artifactData?.artifacts.map((entry) => entry.path)).toEqual(['result/summary.md'])
    } finally {
        vi.useRealTimers()
    }
})

it.each(['spark:runs-transport-reconnect', 'spark:run-resync-required'])(
    'recovers a result that materialized while disconnected on %s',
    async (signal) => {
        const backend = stubMaterializingBackend()
        backend.status = 'completed'
        const { result } = renderHook(() => useRunDetailResources({ selectedRunId: 'run' }))
        await waitFor(() => expect(result.current.resultData?.state).toBe('unavailable'))
        backend.materialized = true
        act(() => { window.dispatchEvent(new CustomEvent(signal, { detail: { runId: 'run' } })) })
        await waitFor(() => expect(result.current.resultData?.body_markdown).toBe('Shipped it.'))
        await waitFor(() => expect(result.current.artifactData?.artifacts).toHaveLength(1))
    },
)

it.each([
    ['CheckpointSaved', null],
    ['PipelineCompleted', null],
    [null, 'spark:run-resync-required'],
    [null, 'spark:runs-transport-reconnect'],
])('keeps context, copy and export current after journal %s / signal %s', async (journalType, signal) => {
    let context: Record<string, unknown> = { 'context.review.findings': 'missing test' }
    vi.stubGlobal('fetch', vi.fn((url: string) => url.endsWith('/run/context')
        ? Promise.resolve(Response.json({ pipeline_id: 'run', context }))
        : new Promise<Response>(() => {})))
    const writeText = vi.fn(() => Promise.resolve())
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { result } = renderHook(() => useRunDetails({ selectedRunSummary: { run_id: 'run' } as RunRecord }))
    await waitFor(() => expect(result.current.contextData?.context).toEqual(context))
    // A later visit clears the finding.
    context = {}
    act(() => {
        window.dispatchEvent(journalType
            ? new CustomEvent('spark:run-journal-entry', { detail: { runId: 'run', entry: { type: journalType } } })
            : new CustomEvent(signal!, { detail: { runId: 'run' } }))
    })
    await waitFor(() => expect(result.current.contextData?.context).toEqual({}))
    expect(decodeURIComponent(result.current.contextExportHref)).not.toContain('missing test')
    await act(() => result.current.copyContextToClipboard())
    expect(writeText).not.toHaveBeenCalled()
    expect(result.current.contextCopyStatus).toBe('No context entries available to copy.')
})
