import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { useRunDetailResources } from '../hooks/useRunDetailResources'
import { useRunTimeline } from '../hooks/useRunTimeline'
import type { PendingInterviewGate, PendingQuestionSnapshot } from '../model/shared'
import { useRunJournalStore } from '../state/runJournalStore'

const pending: { url: string; resolve: (response: Response) => void }[] = []
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } })
const select = (runId: string) => useStore.getState().setRunsSelectedRunIdForScope('all', runId)
const question: PendingQuestionSnapshot = { questionId: 'q', nodeId: 'review', prompt: 'Continue?', questionType: 'YES_NO', options: [] }
const gate = { ...question, eventId: 'q', sequence: 1, receivedAt: '', stageIndex: 0, sourceScope: 'root', sourceParentNodeId: null, sourceFlowName: null, details: null } as PendingInterviewGate

beforeEach(() => {
    pending.length = 0
    useStore.setState(useStore.getInitialState(), true)
    useRunJournalStore.setState({ byRunId: {} })
    useStore.getState().updateRunsListSession({ scopeMode: 'all' })
    select('a')
    vi.stubGlobal('fetch', vi.fn((url: string) => new Promise<Response>((resolve) => pending.push({ url, resolve }))))
})
afterEach(() => vi.unstubAllGlobals())

it('keeps cached resources visible on refresh failure and ignores responses after removal', async () => {
    const cached = { pipeline_id: 'a', context: { cached: 'value' } }
    useStore.getState().updateRunDetailSession('a', { contextData: cached, contextStatus: 'ready' })
    const { result } = renderHook(() => useRunDetailResources({ selectedRunId: 'a', manageSync: false }))
    act(() => { void result.current.fetchContext() })
    expect(result.current.contextData).toBe(cached)
    await act(async () => pending[0].resolve(json({}, 503)))
    expect(result.current.contextData).toBe(cached)
    expect(result.current.contextStatus).toBe('error')
    act(() => { void result.current.fetchContext() })
    act(() => { useStore.getState().clearRunDetailSession('a'); select('b') })
    await act(async () => pending[1].resolve(json({ pipeline_id: 'a', context: { late: true } })))
    expect(useStore.getState().runDetailSessionsByRunId.a).toBeUndefined()
    expect(useStore.getState().runDetailSessionsByRunId.b.contextData).toBeNull()
})

it('rejects obsolete A responses after A → B → A, including errors', async () => {
    const { result, rerender } = renderHook(({ runId }) => useRunDetailResources({ selectedRunId: runId, manageSync: false }), { initialProps: { runId: 'a' } })
    act(() => { void result.current.fetchContext() })
    act(() => select('b'))
    rerender({ runId: 'b' })
    act(() => { void result.current.fetchContext() })
    act(() => select('a'))
    rerender({ runId: 'a' })
    act(() => { void result.current.fetchContext() })
    await act(async () => pending[2].resolve(json({ pipeline_id: 'a', context: { latest: true } })))
    await act(async () => pending[0].resolve(json({}, 503)))
    expect(result.current.contextData?.context).toEqual({ latest: true })
    expect(result.current.contextStatus).toBe('ready')
    expect(result.current.contextError).toBeNull()
})

it('keeps stale questions visible but enforces fresh confirmation in the submission handler', async () => {
    useStore.getState().updateRunDetailSession('a', { questionsStatus: 'ready', pendingQuestionSnapshots: [question] })
    select('b')
    select('a')
    const { result } = renderHook(() => useRunTimeline({ selectedRunTimelineId: 'a', selectedRunCurrentNode: 'review', pendingQuestionSnapshots: [question] }))
    expect(result.current.visiblePendingInterviewGates).toHaveLength(1)
    expect(result.current.confirmedQuestionIds).toEqual([])
    await act(async () => result.current.submitPendingGateAnswer(gate, 'yes'))
    expect(pending).toHaveLength(0)
    act(() => useStore.getState().updateRunDetailSession('a', { questionsStatus: 'error' }))
    await act(async () => result.current.submitPendingGateAnswer(gate, 'yes'))
    expect(pending).toHaveLength(0)
    act(() => useStore.getState().updateRunDetailSession('a', { questionsStatus: 'ready' }))
    const staleHandler = result.current.submitPendingGateAnswer
    act(() => select('b'))
    await act(async () => staleHandler(gate, 'yes'))
    expect(pending).toHaveLength(0)
    act(() => { select('a'); useStore.getState().updateRunDetailSession('a', { questionsStatus: 'ready' }) })
    act(() => { void result.current.submitPendingGateAnswer(gate, 'yes') })
    expect(pending).toHaveLength(1)
    act(() => { useStore.getState().clearRunDetailSession('a'); select('b') })
    await act(async () => pending[0].resolve(json({ status: 'accepted', pipeline_id: 'a', question_id: 'q' })))
    expect(useStore.getState().runDetailSessionsByRunId.a).toBeUndefined()
    expect(useStore.getState().runDetailSessionsByRunId.b.answeredGateIds).toEqual({})
})

it('does not rerender resource subscribers when an unrelated session changes', () => {
    let renders = 0
    renderHook(() => { renders++; return useRunDetailResources({ selectedRunId: 'a', manageSync: false }) })
    const before = renders
    act(() => { select('b'); useStore.getState().updateRunDetailSession('b', { contextSearchQuery: 'other' }) })
    expect(renders).toBe(before)
})
