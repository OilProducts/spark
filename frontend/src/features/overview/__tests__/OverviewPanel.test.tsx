import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AttentionItem } from '@/lib/api/attentionApi'
import type { Mission } from '@/features/missions/MissionsPanel'
import type { RunRecord } from '@/features/runs/model/shared'
import { useStore } from '@/store'
import { OverviewPanel } from '../OverviewPanel'
import { useOverviewUnread } from '../hooks/useOverviewUnread'
import { hasUnread, markAttentionSeen, readFinishedWatermark, type Chat } from '../model/overviewModel'

let pending: AttentionItem[] = []
vi.mock('@/lib/api/attentionApi', () => ({ fetchPendingAttention: () => Promise.resolve(pending) }))
let chats: Chat[] = []
vi.mock('@/lib/workspaceClient', async (original) => ({ ...await original<object>(), fetchProjectConversationListValidated: () => Promise.resolve(chats) }))

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

describe('Overview dot', () => {
    it('stays off while the Overview is open, and shows elsewhere for what it has not shown', () => {
        window.localStorage.clear()
        const item = gate('run-a', 'gate-1')
        act(() => { useStore.getState().setViewMode('overview') })
        const { result, rerender } = renderHook(() => useOverviewUnread([item]))
        expect(result.current).toBe(false)
        act(() => { useStore.getState().setViewMode('runs') })
        rerender()
        expect(result.current).toBe(true)
    })
})

const finishedRun = (id: string, hoursAgo: number): RunRecord => ({
    run_id: id, flow_name: 'review.yaml', status: 'completed', working_directory: '/work/app', project_path: '/work/app', model: 'gpt',
    started_at: new Date(Date.now() - (hoursAgo + 1) * 3_600_000).toISOString(), ended_at: new Date(Date.now() - hoursAgo * 3_600_000).toISOString(),
})
const closedMission = (id: string, hoursAgo: number): Mission => ({
    id, project_path: '/work/app', revision: 1, fields: { title: id, description: '', archived: false }, activity: [], status: 'closed',
    closed: { status: 'done', reason: '', at: new Date(Date.now() - hoursAgo * 3_600_000).toISOString() },
})
// A live upsert fills the list without fetching it; a full fetch marks it fetched.
const setRuns = (runs: RunRecord[], fetched = true) => act(() => { useStore.setState((state) => ({ runsListSession: { ...state.runsListSession, runs, status: 'ready', fetched } })) })
const setMissions = (missions: Mission[], loaded = true) => act(() => { useStore.setState({ missionBoard: missions, missionBoardLoaded: loaded }) })
const finished = () => screen.queryAllByTestId('overview-finished-item').map((row) => row.dataset.itemId)
const reset = () => {
    pending = []
    chats = []
    window.localStorage.clear()
    setRuns([], false)
    setMissions([], false)
}

describe('Overview finished since', () => {
    it('lists the last day on the first visit, then only what it has not shown, live updates included', () => {
        reset()
        setRuns([finishedRun('run-week', 24 * 7), finishedRun('run-hour', 1)])
        setMissions([])
        const first = render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('in the last day')
        expect(finished()).toEqual(['run-hour'])
        setRuns([finishedRun('run-week', 24 * 7), finishedRun('run-hour', 1), finishedRun('run-live', 0)])
        expect(finished()).toEqual(['run-live', 'run-hour'])
        first.unmount()
        const runs = useStore.getState().runsListSession.runs
        expect(readFinishedWatermark()).toBe(Date.parse(runs[2].ended_at!))

        setRuns([...runs, finishedRun('run-after', 0)])
        render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('since you last looked')
        expect(finished()).toEqual(['run-after'])
    })

    it('records nothing while missions are loading, so an earlier mission that loads later is still new', () => {
        reset()
        setRuns([finishedRun('run-new', 1)])
        render(<OverviewPanel />).unmount()
        expect(readFinishedWatermark()).toBeNull()

        setMissions([closedMission('mission-earlier', 2)])
        render(<OverviewPanel />)
        expect(finished()).toEqual(['run-new', 'mission-earlier'])
    })

    it('shows each row with its duration, a word only for failures, where it came from, and the chat it was launched from', async () => {
        reset()
        const chat: Chat = { conversation_id: 'chat-1', project_path: '/work/app', title: 'Ship the fix', created_at: '', updated_at: new Date(Date.now() - 3 * 3_600_000).toISOString(), revision: 1, launched_run_ids: ['run-done', 'run-failed'] }
        chats = [chat]
        act(() => { useStore.getState().hydrateProjectRegistry([{ directoryPath: '/work/app', displayName: 'app', isDefault: false, isFavorite: false, lastAccessedAt: null }]) })
        setRuns([finishedRun('run-done', 1), { ...finishedRun('run-failed', 2), status: 'failed' }])
        setMissions([])
        render(<OverviewPanel />)
        await waitFor(() => expect(screen.getByTestId('overview-last-chat-title').textContent).toBe('Ship the fix'))

        expect(finished()).toEqual(['run-done', 'run-failed'])
        const [done, failed] = screen.getAllByTestId('overview-finished-item')
        expect(done.textContent).toContain('1h 0m')
        expect(done.textContent).toContain('Completed') // the dot's name for assistive tech
        expect(done.textContent).not.toContain('Failed')
        expect(failed.textContent?.match(/Failed/g)).toHaveLength(1) // the visible word is the only one
        expect(failed.querySelector('[data-mark="failed"]')).not.toBeNull()
        expect(failed.textContent).toContain('Run · Review · app')
        expect(failed.textContent).toContain('Failed')
        expect(failed.querySelector('[data-testid="overview-source-link"]')?.textContent).toBe('Ship the fix')
        expect(screen.getByTestId('overview-finished').textContent).toContain('2 · 1 failed')

        expect(screen.getAllByTestId('overview-tally').map((tally) => tally.textContent)).toEqual([' · 1 completed', ' · 1 failed'])
        fireEvent.click(screen.getByTestId('overview-show-runs'))
        const started = screen.getAllByTestId('overview-started-item').map((row) => row.textContent)
        expect(started).toHaveLength(2)
        expect(started[1]).toContain('Failed')
    })

    it('records nothing for live completions before the initial fetches finish', () => {
        reset()
        const run = finishedRun('run-live', 1)
        const mission = closedMission('mission-live', 2)
        setRuns([run], false)
        setMissions([mission], false)
        render(<OverviewPanel />).unmount()
        expect(readFinishedWatermark()).toBeNull()

        setMissions([mission])
        render(<OverviewPanel />).unmount()
        expect(readFinishedWatermark()).toBeNull()

        setRuns([run])
        render(<OverviewPanel />).unmount()
        expect(readFinishedWatermark()).toBe(Date.parse(run.ended_at!))
    })
})
