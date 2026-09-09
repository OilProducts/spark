import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { useRunDetailResources } from '../hooks/useRunDetailResources'

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
                selectedRunCurrentNode: 'work', selectedRunTimelineId: 'run',
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
        const restoredHistory = result.current.groupedTimelineEntries
        expect(JSON.stringify(restoredHistory)).toContain('Custom answer')
        expect(JSON.stringify(restoredHistory)).toContain('Question child')
        // No closing interview event arrives for a dead child request.
        questions = []
        await act(async () => window.dispatchEvent(new CustomEvent('spark:run-upsert', {
            detail: { run: { run_id: 'child', status: 'failed' } },
        })))
        expect(ids()).toEqual([])
        expect(result.current.confirmedQuestionIds).toEqual([])
        expect(result.current.groupedTimelineEntries).toEqual(restoredHistory)
    },
)
