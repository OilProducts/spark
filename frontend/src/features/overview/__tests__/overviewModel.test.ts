import { beforeEach, describe, expect, it } from 'vitest'

import type { Mission } from '@/features/missions/MissionsPanel'
import type { RunRecord } from '@/features/runs/model/shared'
import type { AttentionItem } from '@/lib/api/attentionApi'
import {
    finishedSince,
    hasUnread,
    lastChat,
    markAttentionSeen,
    markSeen,
    readSeenAt,
    sourceOf,
    tally,
    windowStart,
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
        expect(readSeenAt()).toBeNull()
        const since = windowStart(readSeenAt(), now)
        expect(since).toBe(at('2026-10-01T12:00:00Z'))
        const finished = finishedSince(runs, missions, since)
        expect(finished.map((item) => [item.id, item.mark])).toEqual([
            ['closed', 'failed'], ['run-failed', 'failed'], ['run-done', 'completed'],
        ])
    })

    it('starts at your last look once you have looked, and each look moves it', () => {
        markSeen(at('2026-10-02T09:30:00Z'))
        expect(readSeenAt()).toBe(at('2026-10-02T09:30:00Z'))
        expect(finishedSince(runs, missions, windowStart(readSeenAt(), now)).map((item) => item.id)).toEqual(['closed', 'run-failed'])
        expect(hasUnread([], runs, missions, readSeenAt(), now)).toBe(true)

        markSeen(now)
        expect(finishedSince(runs, missions, windowStart(readSeenAt(), now))).toEqual([])
        expect(hasUnread([], runs, missions, readSeenAt(), now)).toBe(false)
        const question: AttentionItem = { kind: 'run_gate', id: 'gate', title: 'Approve?', project_path: '/work/app', run_id: 'run-live', updated_at: '2026-10-02T12:05:00Z' }
        expect(hasUnread([question], runs, missions, readSeenAt(), now)).toBe(true)
    })
})

describe('needs you dot', () => {
    beforeEach(() => window.localStorage.clear())
    const question = (id: string, updatedAt: string): AttentionItem => ({ kind: 'mission', id, title: id, project_path: '/work/app', updated_at: updatedAt })

    it('shows for attention the Overview has not shown, however old its timestamp, and clears once shown', () => {
        const now = at('2026-10-02T12:00:00Z')
        const seen = question('seen', '2026-10-02T11:00:00Z')
        markSeen(now)
        markAttentionSeen([seen])
        expect(hasUnread([seen], [], [], readSeenAt(), now)).toBe(false)

        const older = question('older', '2026-09-01T08:00:00Z')
        expect(hasUnread([seen, older], [], [], readSeenAt(), now)).toBe(true)

        markAttentionSeen([seen, older])
        expect(hasUnread([seen, older], [], [], readSeenAt(), now)).toBe(false)
        expect(hasUnread([older], [], [], readSeenAt(), now)).toBe(false)
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
