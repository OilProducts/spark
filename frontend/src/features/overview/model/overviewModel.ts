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

/** Top-level runs and missions that finished after the watermark, newest first; with no watermark yet, those of the last day. */
export function finishedSince(runs: RunRecord[], missions: Mission[], watermark: number | null, now: number): Started[] {
    const after = watermark ?? now - DAY_MS
    return finished(runs, missions)
        .filter((item) => item.at > after)
        .sort((left, right) => right.at - left.at)
}

const finished = (runs: RunRecord[], missions: Mission[]) => [
    ...runs.filter((run) => !run.parent_run_id)
        .map((run): Started => ({ kind: 'run', id: run.run_id, run, mark: runMark(run), at: timeOf(run.ended_at) }))
        .filter((item) => item.mark !== 'running' && item.mark !== 'waiting'),
    ...missions.filter((mission) => mission.status === 'closed')
        .map((mission): Started => ({ kind: 'mission', id: mission.id, mission, mark: missionMark(mission), at: timeOf(mission.closed?.at) })),
]

/** Whether something needs you or has finished after what the Overview has shown. */
export function hasUnread(attention: AttentionItem[], runs: RunRecord[], missions: Mission[], now: number, attentionWatermark = readAttentionWatermark(), finishedWatermark = readFinishedWatermark()): boolean {
    return attention.some((item) => attentionWatermark === null || timeOf(item.updated_at) > attentionWatermark)
        || finishedSince(runs, missions, finishedWatermark, now).length > 0
}

// The newest server time the Overview has shown in each section; null before the first visit.
// ponytail: an item that finished (or was updated) earlier than one already shown but reached its list later is not new.
const ATTENTION_WATERMARK_KEY = 'spark.overview_attention_watermark'
const FINISHED_WATERMARK_KEY = 'spark.overview_finished_watermark'
const listeners = new Set<() => void>()

function readWatermark(storageKey: string): number | null {
    try {
        const value = window.localStorage.getItem(storageKey)
        return value === null ? null : Number(value) || 0
    } catch {
        return null
    }
}

/** Moves a watermark forward to the newest of the given times; never back, and not at all without times. */
function advance(storageKey: string, times: number[]) {
    const current = readWatermark(storageKey)
    const newest = Math.max(...times)
    if (times.length === 0 || (current !== null && newest <= current)) return
    try {
        window.localStorage.setItem(storageKey, String(newest))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
    listeners.forEach((listener) => listener())
}

export const readAttentionWatermark = () => readWatermark(ATTENTION_WATERMARK_KEY)
export const readFinishedWatermark = () => readWatermark(FINISHED_WATERMARK_KEY)

/** Records the newest pending attention the Overview is showing. */
export function markAttentionSeen(attention: AttentionItem[]) {
    advance(ATTENTION_WATERMARK_KEY, attention.map((item) => timeOf(item.updated_at)))
}

/** Records the newest finish the Overview is showing; on a first visit that also covers those older than its day. */
export function markFinishedSeen(runs: RunRecord[], missions: Mission[]) {
    advance(FINISHED_WATERMARK_KEY, finished(runs, missions).map((item) => item.at))
}

export function subscribeWatermarks(listener: () => void) {
    listeners.add(listener)
    return () => { listeners.delete(listener) }
}
