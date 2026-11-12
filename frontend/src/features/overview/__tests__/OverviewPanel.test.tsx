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
let attentionDown = false
vi.mock('@/lib/api/attentionApi', async (original) => ({
    ...await original<typeof import('@/lib/api/attentionApi')>(),
    fetchPendingAttention: () => (attentionDown ? Promise.reject(new Error('503')) : Promise.resolve(pending)),
}))
let chats: Chat[] = []
// The project path of each thread-list request; the Overview asks once for every project.
let chatListRequests: (string | null)[] = []
vi.mock('@/lib/workspaceClient', async (original) => {
    const client = await original<typeof import('@/lib/workspaceClient')>()
    const { summarySnapshot } = await import('@/test/conversationWire')
    return {
        ...client,
        fetchProjectConversationListValidated: (projectPath: string | null) => {
            chatListRequests.push(projectPath)
            return Promise.resolve(client.parseConversationSummarySnapshotResponse(summarySnapshot(chats)))
        },
    }
})

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

    it('says what each gate asks and when, in place of the kind', async () => {
        pending = [{ ...gate('run-a', 'gate-1'), question: 'Ship v1.2.0?' }, gate('run-b', 'gate-1')]
        render(<OverviewPanel />)
        await waitFor(() => expect(rows()).toEqual(['run-a', 'run-b']))
        const [asked, plain] = screen.getAllByTestId('overview-needs-you-item')
        expect(asked).toHaveTextContent('Ship v1.2.0? · app')
        expect(plain).toHaveTextContent('Run waiting for input · app')
        expect(screen.getAllByTestId('overview-needs-you-time')[0]).toHaveTextContent(/^Oct 2/)
    })
})

describe('Overview needs you, when the check fails', () => {
    it('says it could not check instead of saying nothing is waiting, and recovers on Try again', async () => {
        pending = [gate('run-a', 'gate-1')]
        render(<OverviewPanel />)
        await waitFor(() => expect(rows()).toEqual(['run-a']))

        attentionDown = true
        act(() => { window.dispatchEvent(new Event('focus')) })
        expect(await screen.findByTestId('overview-needs-you-error')).toHaveTextContent('Couldn’t refresh')
        expect(rows()).toEqual(['run-a']) // the last known item stays
        expect(screen.queryByTestId('overview-needs-you-empty')).toBeNull()

        attentionDown = false
        pending = []
        fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
        await waitFor(() => expect(screen.queryByTestId('overview-needs-you-error')).toBeNull())
        expect(screen.getByTestId('overview-needs-you-empty')).toHaveTextContent('Nothing is waiting on you.')
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

// Each run has its own title; runs sharing one fold into a row.
const finishedRun = (id: string, hoursAgo: number): RunRecord => ({
    run_id: id, title: id, flow_name: 'review.yaml', status: 'completed', working_directory: '/work/app', project_path: '/work/app', model: 'gpt',
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
    chatListRequests = []
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
        // A first look has nothing to compare with, so nothing is marked new.
        expect(screen.queryAllByTestId('overview-finished-new')).toHaveLength(0)
        setRuns([finishedRun('run-week', 24 * 7), finishedRun('run-hour', 1), finishedRun('run-live', 0)])
        expect(finished()).toEqual(['run-live', 'run-hour'])
        first.unmount()
        const runs = useStore.getState().runsListSession.runs
        expect(readFinishedWatermark()).toBe(Date.parse(runs[2].ended_at!))

        // A later look keeps the last day listed, marking only what finished since as new.
        setRuns([...runs, finishedRun('run-after', 0)])
        const second = render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('in the last day')
        expect(finished()).toEqual(['run-after', 'run-live', 'run-hour'])
        expect(screen.getAllByTestId('overview-finished-new')).toHaveLength(1)
        second.unmount()

        // After a long time away, the list reaches back to the last look.
        window.localStorage.setItem('spark.overview_finished_watermark', String(Date.now() - 24 * 8 * 3_600_000))
        render(<OverviewPanel />)
        expect(screen.getByTestId('overview-finished').textContent).toContain('since you last looked')
        expect(finished()).toEqual(['run-after', 'run-live', 'run-hour', 'run-week'])
    })

    it('lists running runs, missions and chats under Running now, and not as finished', () => {
        reset()
        setRuns([{ ...finishedRun('run-going', 0), status: 'running', ended_at: null }, { ...finishedRun('run-child', 0), status: 'running', ended_at: null, parent_run_id: 'run-going' }])
        setMissions([{ ...closedMission('mission-going', 0), status: 'running', closed: undefined }])
        render(<OverviewPanel />)
        expect(screen.getAllByTestId('overview-running-item').map((row) => row.dataset.itemId).sort()).toEqual(['mission-going', 'run-going'])
        expect(finished()).toEqual([])
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
        // Titled by their flow; a completed and a failed run of one flow keep their own rows.
        setRuns([{ ...finishedRun('run-done', 1), title: null }, { ...finishedRun('run-failed', 2), status: 'failed', title: null }])
        setMissions([])
        render(<OverviewPanel />)
        await waitFor(() => expect(screen.getByTestId('overview-last-chat-title').textContent).toBe('Ship the fix'))
        expect(chatListRequests).toEqual([null])

        // Failures lead.
        expect(finished()).toEqual(['run-failed', 'run-done'])
        const [failed, done] = screen.getAllByTestId('overview-finished-item')
        expect(done.textContent).toContain('1h 00m')
        expect(done.textContent).toContain('Completed') // the dot's name for assistive tech
        expect(done.textContent).not.toContain('Failed')
        expect(failed.textContent?.match(/Failed/g)).toHaveLength(1) // the visible word is the only one
        expect(failed.querySelector('[data-mark="failed"]')).not.toBeNull()
        expect(failed.textContent).toContain('Run · app') // the title already names the flow
        expect(failed.textContent).toContain('Failed')
        expect(failed.querySelector('[data-testid="overview-source-link"]')?.textContent).toBe('Ship the fix')
        expect(screen.getByTestId('overview-finished').textContent).toContain('in the last day2')
        expect(screen.getByTestId('overview-failure-count')).toHaveTextContent('1 failed')

        expect(screen.getAllByTestId('overview-tally').map((tally) => tally.textContent)).toEqual([' · 1 completed', ' · 1 failed'])
        fireEvent.click(screen.getByTestId('overview-show-runs'))
        const started = screen.getAllByTestId('overview-started-item').map((row) => row.textContent)
        expect(started).toHaveLength(2)
        expect(started[1]).toContain('Failed')
    })

    it('folds a routine run\'s repeats into one row, keeps failures first, shows the rest on request, and links failures to Runs', () => {
        reset()
        const routine = (id: string, hoursAgo: number, status = 'completed'): RunRecord => ({ ...finishedRun(id, hoursAgo), title: 'Nightly sync', status })
        setRuns([
            ...Array.from({ length: 30 }, (_, index) => routine(`sync-${index}`, 1 + index / 2)),
            routine('sync-failed', 20, 'failed'),
            ...Array.from({ length: 9 }, (_, index) => finishedRun(`other-${index}`, 2 + index)),
        ])
        setMissions([])
        act(() => { useStore.getState().setViewMode('overview') })
        render(<OverviewPanel />)
        const rows = screen.getAllByTestId('overview-finished-item')
        expect(rows).toHaveLength(8)
        expect(rows[0].dataset.itemId).toBe('sync-failed')
        expect(rows[1].dataset.itemId).toBe('sync-0')
        expect(rows[1].dataset.count).toBe('30')
        expect(rows[1].textContent).toContain('30 runs, latest shown')
        fireEvent.click(screen.getByTestId('overview-finished-show-all'))
        expect(screen.getAllByTestId('overview-finished-item')).toHaveLength(11)

        fireEvent.click(screen.getByTestId('overview-failure-count'))
        expect(useStore.getState().viewMode).toBe('runs')
        expect(useStore.getState().runsListSession.statusFilter).toBe('failed')
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
