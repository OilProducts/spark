import { beforeEach, describe, expect, it } from 'vitest'

import type { Mission } from '@/features/missions/MissionsPanel'
import type { RunRecord } from '@/features/runs/model/shared'
import type { AttentionItem } from '@/lib/api/attentionApi'
import {
    finishedSince,
    hasUnread,
    lastChat,
    forgetRemovedFinished,
    forgetResolvedAttention,
    markAttentionSeen,
    markFinishedSeen,
    readSeenFinished,
    sourceOf,
    tally,
    type Chat,
} from '../model/overviewModel'

const chat = (id: string, updatedAt: string, launched: string[] = []): Chat => ({
    conversation_id: id, project_path: '/work/app', title: id, created_at: updatedAt, updated_at: updatedAt, revision: 1, launched_run_ids: launched,
})
const run = (id: string, status: string, startedAt: string, endedAt: string | null = null, extra: Partial<RunRecord> = {}): RunRecord => ({
    run_id: id, flow_name: 'team/review.yaml', status, working_directory: '/work/app', project_path: '/work/app', model: 'gpt', started_at: startedAt, ended_at: endedAt, ...extra,
})
const mission = (id: string, extra: Partial<Mission> = {}): Mission => ({
    id, project_path: '/work/lib', revision: 1, fields: { title: id, description: '', archived: false }, activity: [], status: 'running', ...extra,
})
const at = (value: string) => Date.parse(value)

describe('you were last in', () => {
    it('counts a chat\'s launched runs and missions as its activity, not only its own messages', () => {
        const chats = [
            chat('talked-last', '2026-10-01T09:00:00Z'),
            chat('launched', '2026-10-01T08:00:00Z', ['run-late', 'run-missing']),
        ]
        const runs = [run('run-late', 'completed', '2026-10-01T08:05:00Z', '2026-10-01T10:00:00Z')]
        const last = lastChat(chats, runs, [])
        expect(last?.chat.conversation_id).toBe('launched')
        expect(last?.at).toBe(at('2026-10-01T10:00:00Z'))
        // A launch whose run is gone shows nothing for it.
        expect(last?.started.map((item) => item.id)).toEqual(['run-late'])

        // A newer message in the other chat wins again.
        expect(lastChat([chat('talked-last', '2026-10-01T11:00:00Z'), chats[1]], runs, [])?.chat.conversation_id).toBe('talked-last')
    })

    it('counts a mission created from the chat, using its mission-format times, and tallies what it started by state', () => {
        const chats = [chat('quiet', '2026-10-01T09:00:00Z', ['run-a', 'run-b']), chat('other', '2026-10-01T10:00:00Z')]
        const runs = [
            run('run-a', 'failed', '2026-10-01T08:00:00Z', '2026-10-01T08:30:00Z'),
            run('run-b', 'running', '2026-10-01T08:40:00Z'),
        ]
        const missions = [
            mission('from-quiet', { source_conversation_id: 'quiet', status: 'closed', updated_at: '2026-10-01 11:00:00.0 +00:00:00', closed: { status: 'done', reason: '', at: '2026-10-01 11:00:00.0 +00:00:00' } }),
            mission('unrelated', { updated_at: '2026-10-01 12:00:00.0 +00:00:00' }),
        ]
        const last = lastChat(chats, runs, missions)
        expect(last?.chat.conversation_id).toBe('quiet')
        expect(last?.started.map((item) => item.id)).toEqual(['from-quiet', 'run-b', 'run-a'])
        expect(tally(last!.started)).toEqual({ completed: 1, running: 1, failed: 1 })
    })

    it('counts runs on the roster of a mission the chat started, once even when the chat launched them too', () => {
        const chats = [chat('source', '2026-10-01T08:00:00Z', ['run-roster']), chat('other', '2026-10-01T10:00:00Z')]
        const runs = [
            run('run-roster', 'completed', '2026-10-01T09:30:00Z', '2026-10-01T11:00:00Z'),
            run('run-roster-2', 'failed', '2026-10-01T09:00:00Z', '2026-10-01T09:10:00Z'),
        ]
        const missions = [mission('from-source', {
            source_conversation_id: 'source', updated_at: '2026-10-01 09:00:00.0 +00:00:00',
            runs: ['run-roster', 'run-roster-2'].map((run_id) => ({ run_id, flow_name: 'f', summary: '', launched_at: '', status: 'completed' })),
        })]
        const last = lastChat(chats, runs, missions)
        expect(last?.chat.conversation_id).toBe('source')
        expect(last?.at).toBe(at('2026-10-01T11:00:00Z'))
        expect(last?.started.map((item) => item.id)).toEqual(['run-roster', 'run-roster-2', 'from-source'])
        expect(tally(last!.started)).toEqual({ completed: 1, failed: 1, running: 1 })
    })

    it('has nothing to show without chats', () => {
        expect(lastChat([], [run('run-a', 'completed', '2026-10-01T08:00:00Z')], [])).toBeNull()
    })
})

describe('finished since', () => {
    const now = at('2026-10-02T12:00:00Z')
    const runs = [
        run('run-old', 'completed', '2026-09-30T08:00:00Z', '2026-09-30T09:00:00Z'),
        run('run-done', 'completed', '2026-10-02T08:00:00Z', '2026-10-02T09:00:00Z'),
        run('run-failed', 'failed', '2026-10-02T09:00:00Z', '2026-10-02T10:00:00Z'),
        run('run-live', 'running', '2026-10-02T09:00:00Z'),
        run('run-child', 'completed', '2026-10-02T09:00:00Z', '2026-10-02T11:00:00Z', { parent_run_id: 'run-done' }),
    ]
    const missions = [mission('closed', { status: 'closed', closed: { status: 'failed', reason: 'x', at: '2026-10-02 11:30:00.0 +00:00:00' } })]

    beforeEach(() => window.localStorage.clear())

    it('falls back to the last day on the first visit, lists top-level finishes newest first and marks failures', () => {
        expect(readSeenFinished()).toBeNull()
        const finished = finishedSince(runs, missions, readSeenFinished(), now)
        expect(finished.map((item) => [item.id, item.mark])).toEqual([
            ['closed', 'failed'], ['run-failed', 'failed'], ['run-done', 'completed'],
        ])
        expect(hasUnread([], runs, missions, now)).toBe(true)
    })

    it('stops listing what was shown, older finishes included, and lists what finished after the visit', () => {
        markFinishedSeen(runs, missions)
        expect(finishedSince(runs, missions, readSeenFinished(), now)).toEqual([])
        expect(hasUnread([], runs, missions, now)).toBe(false)

        const later = [...runs.filter((item) => item.run_id !== 'run-live'), run('run-live', 'completed', '2026-10-02T09:00:00Z', '2026-10-02T12:30:00Z')]
        expect(finishedSince(later, missions, readSeenFinished(), now).map((item) => item.id)).toEqual(['run-live'])
        expect(hasUnread([], later, missions, now)).toBe(true)
        markFinishedSeen(later, missions)
        expect(hasUnread([], later, missions, now)).toBe(false)
    })

    it('lists a shown failure again when a retry under the same id finishes later, failed or completed', () => {
        markFinishedSeen(runs, missions)
        const retry = (status: string, endedAt: string | null) =>
            runs.map((item) => (item.run_id === 'run-failed' ? run('run-failed', status, '2026-10-02T11:00:00Z', endedAt) : item))
        // Retried while away: running is not finished, and keeps the shown version.
        markFinishedSeen(retry('running', null), missions)
        for (const status of ['failed', 'completed']) {
            const done = retry(status, status === 'failed' ? '2026-10-02T11:45:00Z' : '2026-10-02T11:50:00Z')
            expect(finishedSince(done, missions, readSeenFinished(), now).map((item) => [item.id, item.mark])).toEqual([['run-failed', status]])
            expect(hasUnread([], done, missions, now)).toBe(true)
            markFinishedSeen(done, missions)
            expect(finishedSince(done, missions, readSeenFinished(), now)).toEqual([])
            expect(hasUnread([], done, missions, now)).toBe(false)
        }
        // The newer version replaced the older one.
        expect(readSeenFinished()?.split('\n').filter((key) => key.startsWith('run:run-failed@'))).toHaveLength(1)
    })

    it('marks nothing before the lists load and keeps what was seen through a partial live list until a full fetch', () => {
        markFinishedSeen([], [])
        expect(readSeenFinished()).toBeNull()

        // Seen on an earlier visit; at startup a live completion arrives before the full lists, and you leave.
        markFinishedSeen([runs[1]], [missions[0]])
        const liveRun = run('run-b', 'completed', '2026-10-02T11:00:00Z', '2026-10-02T11:30:00Z')
        const liveMission = mission('m-b', { status: 'closed', closed: { status: 'done', reason: '', at: '2026-10-02 11:40:00.0 +00:00:00' } })
        markFinishedSeen([liveRun], [liveMission])
        forgetRemovedFinished('run', [runs[1].run_id, liveRun.run_id])
        forgetRemovedFinished('mission', [missions[0].id, liveMission.id])
        expect(finishedSince([runs[1], liveRun], [missions[0], liveMission], readSeenFinished(), now)).toEqual([])
        expect(hasUnread([], [runs[1], liveRun], [missions[0], liveMission], now)).toBe(false)
    })

    it('records an empty first visit once both lists loaded, so a completion seen more than a day later is listed', () => {
        const active = [run('run-live', 'running', '2026-10-02T09:00:00Z')]
        forgetRemovedFinished('run', ['run-live'])
        forgetRemovedFinished('mission', [])
        markFinishedSeen(active, [])
        expect(readSeenFinished()).toBe('')

        const done = [run('run-live', 'completed', '2026-10-02T09:00:00Z', '2026-10-02T10:00:00Z')]
        const twoDaysLater = at('2026-10-04T12:00:00Z')
        expect(finishedSince(done, [], readSeenFinished(), twoDaysLater).map((item) => item.id)).toEqual(['run-live'])
        expect(hasUnread([], done, [], twoDaysLater)).toBe(true)
    })

    it('drops a seen item that left its fetched list, the last finished one included', () => {
        markFinishedSeen(runs, missions)
        forgetRemovedFinished('run', runs.filter((item) => item.run_id !== 'run-old').map((item) => item.run_id))
        expect(readSeenFinished()?.split('\n').map((key) => key.split('@')[0]).sort()).toEqual(['mission:closed', 'run:run-done', 'run:run-failed'])
        // Only an active run is left, and the closed mission is gone.
        forgetRemovedFinished('run', ['run-live'])
        forgetRemovedFinished('mission', [])
        expect(readSeenFinished()).toBe('')
    })

    it('keeps fractional seconds of mission times at a same-second boundary', () => {
        const dayLater = at('2026-10-02T12:00:00.100Z')
        const closedAt = (status: 'done' | 'failed', value: string) => mission(`m-${value}`, { status: 'closed', closed: { status, reason: '', at: value } })
        const earlier = closedAt('done', '2026-10-01 12:00:00.05 +00:00:00')
        const later = closedAt('failed', '2026-10-01 12:00:00.8 +00:00:00')
        const run800 = run('run-mid', 'completed', '2026-10-01T11:00:00Z', '2026-10-01T12:00:00.500Z')
        expect(finishedSince([run800], [earlier, later], null, dayLater).map((item) => item.id)).toEqual([later.id, 'run-mid'])
        expect(hasUnread([], [], [earlier], dayLater)).toBe(false)
        expect(hasUnread([], [], [later], dayLater)).toBe(true)
    })
})

describe('needs you dot', () => {
    beforeEach(() => window.localStorage.clear())
    const question = (id: string, updatedAt: string): AttentionItem => ({ kind: 'mission', id, title: id, project_path: '/work/app', updated_at: updatedAt })

    it('shows for attention the Overview has not shown, however old its timestamp, and clears once shown', () => {
        const now = at('2026-10-02T12:00:00Z')
        const seen = question('seen', '2026-10-02T11:00:00Z')
        markAttentionSeen([seen])
        expect(hasUnread([seen], [], [], now)).toBe(false)

        const older = question('older', '2026-09-01T08:00:00Z')
        expect(hasUnread([seen, older], [], [], now)).toBe(true)

        markAttentionSeen([seen, older])
        expect(hasUnread([seen, older], [], [], now)).toBe(false)
        expect(hasUnread([older], [], [], now)).toBe(false)
    })

    it('shows again for a mission that needs you again, seen while the Overview was closed or by its new timestamp', () => {
        const now = at('2026-10-02T12:00:00Z')
        const first = question('mission-a', '2026-10-02T10:00:00Z')
        markAttentionSeen([first])
        forgetResolvedAttention([first])
        expect(hasUnread([first], [], [], now)).toBe(false)

        // A poll misses the gap, but the request carries a new timestamp.
        const again = question('mission-a', '2026-10-02T11:30:00Z')
        forgetResolvedAttention([again])
        expect(hasUnread([again], [], [], now)).toBe(true)
        markAttentionSeen([again])
        expect(hasUnread([again], [], [], now)).toBe(false)
    })

    it('shows again for a successive gate on the same run once a poll saw the first one answered', () => {
        const now = at('2026-10-02T12:00:00Z')
        const gate: AttentionItem = { kind: 'run_gate', id: 'run-a', title: 'flow', project_path: '/work/app', run_id: 'run-a', updated_at: '2026-10-02T09:00:00Z' }
        markAttentionSeen([gate])
        forgetResolvedAttention([])
        expect(hasUnread([], [], [], now)).toBe(false)
        forgetResolvedAttention([gate])
        expect(hasUnread([gate], [], [], now)).toBe(true)
    })

    it('shows again for a second gate on an already-seen run even when no poll saw the first one answered', () => {
        const now = at('2026-10-02T12:00:00Z')
        // A run gate is identified by its run and pending question; the run's start time stays the same.
        const gate = (questionId: string): AttentionItem => ({ kind: 'run_gate', id: `run-a:${questionId}`, title: 'flow', project_path: '/work/app', run_id: 'run-a', updated_at: '2026-10-02T09:00:00Z' })
        markAttentionSeen([gate('gate-1')])
        expect(hasUnread([gate('gate-1')], [], [], now)).toBe(false)

        forgetResolvedAttention([gate('gate-2')])
        expect(hasUnread([gate('gate-2')], [], [], now)).toBe(true)
    })
})

describe('linking work to where it started', () => {
    const chats = [chat('chat-a', '2026-10-01T09:00:00Z', ['run-from-chat'])]
    const missions = [
        mission('mission-a', { source_conversation_id: 'chat-a', runs: [{ run_id: 'run-from-mission', flow_name: 'f', summary: '', launched_at: '', status: 'completed' }] }),
        mission('mission-b'),
    ]

    it('links a run to the chat that launched it or the mission whose roster holds it', () => {
        expect(sourceOf({ kind: 'run', id: 'run-from-chat' }, chats, missions)).toEqual({ kind: 'chat', chat: chats[0] })
        expect(sourceOf({ kind: 'run', id: 'run-from-mission' }, chats, missions)).toEqual({ kind: 'mission', mission: missions[0] })
        expect(sourceOf({ kind: 'run', id: 'run-by-hand' }, chats, missions)).toBeNull()
    })

    it('links a mission to the chat it came from', () => {
        expect(sourceOf({ kind: 'mission', mission: missions[0] }, chats, missions)).toEqual({ kind: 'chat', chat: chats[0] })
        expect(sourceOf({ kind: 'mission', mission: missions[1] }, chats, missions)).toBeNull()
    })
})
