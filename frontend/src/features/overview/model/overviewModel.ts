import type { ConversationSummaryResponse } from '@/lib/api/conversationsApi'
import type { AttentionItem } from '@/lib/api/attentionApi'
import type { Mission } from '@/features/missions/MissionsPanel'
import { missionTime } from '@/features/missions/model/missionModel'
import { runStatusKind } from '@/features/runs/model/runOverviewModel'
import type { RunRecord } from '@/features/runs/model/shared'

// What the Overview shows: the chat you were last in with what it started,
// what finished since you last looked, and whether there is anything new.

export type Chat = ConversationSummaryResponse
export type Mark = 'waiting' | 'running' | 'completed' | 'failed' | 'ended' | 'draft'
/** A run or mission with its state mark and its latest time. */
export type Started =
    | { kind: 'run'; id: string; run: RunRecord; mark: Mark; at: number }
    | { kind: 'mission'; id: string; mission: Mission; mark: Mark; at: number }
export type Source = { kind: 'chat'; chat: Chat } | { kind: 'mission'; mission: Mission }

const DAY_MS = 24 * 60 * 60 * 1000

/** Milliseconds for run and chat ISO times and mission times alike; 0 when missing. */
export const timeOf = (value: string | null | undefined) => Date.parse(missionTime(value)) || 0

export function runMark(run: RunRecord): Mark {
    return runStatusKind(run.status, false)
}

export function missionMark(mission: Mission): Mark {
    switch (mission.status ?? 'draft') {
        case 'needs_you': return 'waiting'
        case 'running': return 'running'
        case 'draft': return 'draft'
        case 'closed': return mission.closed?.status === 'failed' ? 'failed' : mission.closed?.status === 'canceled' ? 'ended' : 'completed'
    }
}

const runAt = (run: RunRecord) => timeOf(run.ended_at) || timeOf(run.started_at)
const missionAt = (mission: Mission) => Math.max(timeOf(mission.closed?.at), timeOf(mission.updated_at), timeOf(mission.started_at))

/** The runs a chat launched, the missions created from it and those missions' roster runs, each once, newest first. */
export function startedBy(chat: Chat, runs: RunRecord[], missions: Mission[]): Started[] {
    const own = missions.filter((mission) => mission.source_conversation_id === chat.conversation_id)
    const launched = new Set([...(chat.launched_run_ids ?? []), ...own.flatMap((mission) => mission.runs?.map((run) => run.run_id) ?? [])])
    return [
        ...runs.filter((run) => launched.has(run.run_id))
            .map((run): Started => ({ kind: 'run', id: run.run_id, run, mark: runMark(run), at: runAt(run) })),
        ...own.map((mission): Started => ({ kind: 'mission', id: mission.id, mission, mark: missionMark(mission), at: missionAt(mission) })),
    ].sort((left, right) => right.at - left.at)
}

/** The chat with the most recent activity: its own messages, and the runs and missions it started. */
export function lastChat(chats: Chat[], runs: RunRecord[], missions: Mission[]) {
    let best: { chat: Chat; at: number; started: Started[] } | null = null
    for (const chat of chats) {
        const started = startedBy(chat, runs, missions)
        const at = Math.max(timeOf(chat.updated_at), ...started.map((item) => item.at))
        if (!best || at > best.at) best = { chat, at, started }
    }
    return best
}

/** How many of what a chat started are in each state. */
export function tally(started: Started[]): Partial<Record<Mark, number>> {
    const counts: Partial<Record<Mark, number>> = {}
    for (const item of started) counts[item.mark] = (counts[item.mark] ?? 0) + 1
    return counts
}

/** The chat that launched a run or the mission whose roster holds it; for a mission, the chat it came from. */
export function sourceOf(item: { kind: 'run'; id: string } | { kind: 'mission'; mission: Mission }, chats: Chat[], missions: Mission[]): Source | null {
    if (item.kind === 'mission') {
        const chat = chats.find((entry) => entry.conversation_id === item.mission.source_conversation_id)
        return chat ? { kind: 'chat', chat } : null
    }
    const chat = chats.find((entry) => entry.launched_run_ids?.includes(item.id))
    if (chat) return { kind: 'chat', chat }
    const mission = missions.find((entry) => entry.runs?.some((run) => run.run_id === item.id))
    return mission ? { kind: 'mission', mission } : null
}

/** Where "finished since" starts: your last look, or a day ago on the first visit. */
export const windowStart = (seenAt: number | null, now: number) => seenAt ?? now - DAY_MS

/** Top-level runs and missions that finished after `since`, newest first. */
export function finishedSince(runs: RunRecord[], missions: Mission[], since: number): Started[] {
    return [
        ...runs.filter((run) => !run.parent_run_id)
            .map((run): Started => ({ kind: 'run', id: run.run_id, run, mark: runMark(run), at: timeOf(run.ended_at) }))
            .filter((item) => item.mark !== 'running' && item.mark !== 'waiting'),
        ...missions.filter((mission) => mission.status === 'closed')
            .map((mission): Started => ({ kind: 'mission', id: mission.id, mission, mark: missionMark(mission), at: timeOf(mission.closed?.at) })),
    ].filter((item) => item.at > since).sort((left, right) => right.at - left.at)
}

// The version keeps a recurring request with a new timestamp from passing as one already shown.
const attentionKey = (item: AttentionItem) => `${item.kind}:${item.id}@${item.updated_at}`

/** Whether something needs you that the Overview has not shown, or something finished since your last look. */
export function hasUnread(attention: AttentionItem[], runs: RunRecord[], missions: Mission[], seenAt: number | null, now: number, seenAttention = readSeenAttention()): boolean {
    const seen = new Set(seenAttention.split('\n'))
    return attention.some((item) => !seen.has(attentionKey(item))) || finishedSince(runs, missions, windowStart(seenAt, now)).length > 0
}

const SEEN_KEY = 'spark.overview_seen_at'
const SEEN_ATTENTION_KEY = 'spark.overview_seen_attention'
const listeners = new Set<() => void>()

/** The attention the Overview last showed, as newline-joined keys; a string so it is a stable snapshot. */
export function readSeenAttention(): string {
    try {
        return window.localStorage.getItem(SEEN_ATTENTION_KEY) ?? ''
    } catch {
        return ''
    }
}

function writeSeenAttention(keys: string[]) {
    try {
        window.localStorage.setItem(SEEN_ATTENTION_KEY, keys.join('\n'))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
    listeners.forEach((listener) => listener())
}

/** Records the attention the Overview is showing; only what is pending now is kept. */
export function markAttentionSeen(attention: AttentionItem[]) {
    writeSeenAttention(attention.map(attentionKey))
}

/** Forgets shown attention that is no longer pending, so if it comes back it is new again. Call with each fetched list. */
export function forgetResolvedAttention(attention: AttentionItem[]) {
    const pending = new Set(attention.map(attentionKey))
    const seen = readSeenAttention().split('\n').filter(Boolean)
    const kept = seen.filter((key) => pending.has(key))
    if (kept.length < seen.length) writeSeenAttention(kept)
}

export function readSeenAt(): number | null {
    try {
        const value = Number(window.localStorage.getItem(SEEN_KEY))
        return value > 0 ? value : null
    } catch {
        return null
    }
}

export function markSeen(now = Date.now()) {
    try {
        window.localStorage.setItem(SEEN_KEY, String(now))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
    listeners.forEach((listener) => listener())
}

export function subscribeSeenAt(listener: () => void) {
    listeners.add(listener)
    return () => { listeners.delete(listener) }
}
