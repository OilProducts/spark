import { act, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AttentionItem } from '@/lib/api/attentionApi'
import type { RunRecord } from '@/features/runs/model/shared'
import { useStore } from '@/store'
import { OverviewPanel } from '../OverviewPanel'
import { hasUnread, markAttentionSeen } from '../model/overviewModel'

let pending: AttentionItem[] = []
vi.mock('@/lib/api/attentionApi', () => ({ fetchPendingAttention: () => Promise.resolve(pending) }))

// Run gate ids as the aggregate emits them: the run id with its run-local question id.
const gate = (runId: string, questionId: string): AttentionItem => ({
    kind: 'run_gate', id: `${runId}:${questionId}`, title: runId, project_path: '/work/app', run_id: runId, updated_at: '2026-10-02T09:00:00Z',
})
const rows = () => screen.queryAllByTestId('overview-needs-you-item').map((row) => row.querySelector('.truncate.text-sm')?.textContent)
const poll = async (next: AttentionItem[], expected: string[]) => {
    pending = next
    act(() => { window.dispatchEvent(new Event('focus')) })
    await waitFor(() => expect(rows()).toEqual(expected))
}

describe('Overview needs you', () => {
    it('keeps two runs at the same gate apart as one advances and resolves', async () => {
        pending = [gate('run-a', 'gate-1'), gate('run-b', 'gate-1')]
        render(<OverviewPanel />)
        await waitFor(() => expect(rows()).toEqual(['run-a', 'run-b']))

        await poll([gate('run-a', 'gate-2'), gate('run-b', 'gate-1')], ['run-a', 'run-b'])
        await poll([gate('run-b', 'gate-1')], ['run-b'])
    })
})

describe('Overview needs you seen', () => {
    it('keeps shown attention seen when the Overview is left before attention loads', () => {
        window.localStorage.clear()
        const item = gate('run-a', 'gate-1')
        markAttentionSeen([item])
        render(<OverviewPanel />).unmount()
        expect(hasUnread([item], [], [], Date.now())).toBe(false)
    })
})

const finishedRun = (id: string, hoursAgo: number): RunRecord => ({
    run_id: id, flow_name: 'review.yaml', status: 'completed', working_directory: '/work/app', project_path: '/work/app', model: 'gpt',
    started_at: new Date(Date.now() - (hoursAgo + 1) * 3_600_000).toISOString(), ended_at: new Date(Date.now() - hoursAgo * 3_600_000).toISOString(),
})
const setRuns = (runs: RunRecord[]) => act(() => { useStore.setState((state) => ({ runsListSession: { ...state.runsListSession, runs } })) })
const finished = () => screen.queryAllByTestId('overview-finished-item').map((row) => row.dataset.itemId)

describe('Overview finished since', () => {
    it('lists the last day on the first visit, then only what it has not shown, live updates included', () => {
        pending = []
        window.localStorage.clear()
        setRuns([])
        // A visit that closes before the lists load sees nothing.
        render(<OverviewPanel />).unmount()

        setRuns([finishedRun('run-week', 24 * 7), finishedRun('run-hour', 1)])
        const first = render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('in the last day')
        expect(finished()).toEqual(['run-hour'])
        setRuns([finishedRun('run-week', 24 * 7), finishedRun('run-hour', 1), finishedRun('run-live', 0)])
        expect(finished()).toEqual(['run-live', 'run-hour'])
        first.unmount()

        const runs = useStore.getState().runsListSession.runs
        setRuns([...runs, finishedRun('run-after', 0)])
        render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('since you last looked')
        expect(finished()).toEqual(['run-after'])
    })
})
