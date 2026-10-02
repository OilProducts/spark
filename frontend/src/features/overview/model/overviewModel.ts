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

// Versioned by completion time, as attention keys are, so a retried run that finishes again is new again.
const finishedKey = (item: Started) => `${item.kind}:${item.id}@${item.at}`
const unversioned = (key: string) => key.slice(0, key.lastIndexOf('@'))

/** Top-level runs and missions that finished and are not yet seen, newest first; with nothing seen yet, those of the last day. */
export function finishedSince(runs: RunRecord[], missions: Mission[], seenFinished: string | null, now: number): Started[] {
    const seen = new Set(seenFinished?.split('\n'))
    return finished(runs, missions)
        .filter((item) => (seenFinished === null ? item.at > now - DAY_MS : !seen.has(finishedKey(item))))
        .sort((left, right) => right.at - left.at)
}

const finished = (runs: RunRecord[], missions: Mission[]) => [
    ...runs.filter((run) => !run.parent_run_id)
        .map((run): Started => ({ kind: 'run', id: run.run_id, run, mark: runMark(run), at: timeOf(run.ended_at) }))
        .filter((item) => item.mark !== 'running' && item.mark !== 'waiting'),
    ...missions.filter((mission) => mission.status === 'closed')
        .map((mission): Started => ({ kind: 'mission', id: mission.id, mission, mark: missionMark(mission), at: timeOf(mission.closed?.at) })),
]

// The version keeps a recurring request with a new timestamp from passing as one already shown.
const attentionKey = (item: AttentionItem) => `${item.kind}:${item.id}@${item.updated_at}`

/** Whether something needs you or has finished that the Overview has not shown. */
export function hasUnread(attention: AttentionItem[], runs: RunRecord[], missions: Mission[], now: number, seenAttention = readSeenAttention(), seenFinished = readSeenFinished()): boolean {
    const seen = new Set(seenAttention.split('\n'))
    return attention.some((item) => !seen.has(attentionKey(item))) || finishedSince(runs, missions, seenFinished, now).length > 0
}

// What the Overview has shown, as newline-joined keys; a string so it is a stable snapshot, null when nothing was ever shown.
const SEEN_ATTENTION_KEY = 'spark.overview_seen_attention'
const SEEN_FINISHED_KEY = 'spark.overview_seen_finished'
const listeners = new Set<() => void>()

function readSeen(storageKey: string): string | null {
    try {
        return window.localStorage.getItem(storageKey)
    } catch {
        return null
    }
}

function writeSeen(storageKey: string, keys: string[]) {
    if (readSeen(storageKey) === keys.join('\n')) return
    try {
        window.localStorage.setItem(storageKey, keys.join('\n'))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
    listeners.forEach((listener) => listener())
}

export const readSeenAttention = () => readSeen(SEEN_ATTENTION_KEY) ?? ''
export const readSeenFinished = () => readSeen(SEEN_FINISHED_KEY)

/** Records the attention the Overview is showing; only what is pending now is kept. */
export function markAttentionSeen(attention: AttentionItem[]) {
    writeSeen(SEEN_ATTENTION_KEY, attention.map(attentionKey))
}

/** Forgets shown attention that is no longer pending, so if it comes back it is new again. Call with each fetched list. */
export function forgetResolvedAttention(attention: AttentionItem[]) {
    const pending = new Set(attention.map(attentionKey))
    const seen = readSeenAttention().split('\n').filter(Boolean)
    const kept = seen.filter((key) => pending.has(key))
    if (kept.length < seen.length) writeSeen(SEEN_ATTENTION_KEY, kept)
}

/**
 * Records every finished run and mission as seen while the Overview shows them: those it lists, and on the first
 * visit those older than its day, so the next visit does not list them all. Once both lists were fetched, records the
 * visit even when nothing has finished; before then, with nothing shown, marks nothing.
 */
export function markFinishedSeen(runs: RunRecord[], missions: Mission[]) {
    const shown = finished(runs, missions).map(finishedKey)
    if (shown.length === 0 && fetched.size < 2) return
    const replaced = new Set(shown.map(unversioned))
    const kept = (readSeenFinished() ?? '').split('\n').filter((key) => key && !replaced.has(unversioned(key)))
    writeSeen(SEEN_FINISHED_KEY, [...kept, ...shown])
}

// The kinds whose full list has been fetched this session.
const fetched = new Set<Started['kind']>()

/** Forgets shown runs or missions that left their list. Call with each fully fetched list, never a live-updated one. */
export function forgetRemovedFinished(kind: Started['kind'], ids: string[]) {
    fetched.add(kind)
    // ponytail: storage stays bounded by the fetched lists; a run that leaves the list and comes back counts as new again.
    const listed = new Set(ids.map((id) => `${kind}:${id}`))
    const seen = (readSeenFinished() ?? '').split('\n').filter(Boolean)
    const kept = seen.filter((key) => !key.startsWith(`${kind}:`) || listed.has(unversioned(key)))
    if (kept.length < seen.length) writeSeen(SEEN_FINISHED_KEY, kept)
}

export function subscribeSeen(listener: () => void) {
    listeners.add(listener)
    return () => { listeners.delete(listener) }
}
