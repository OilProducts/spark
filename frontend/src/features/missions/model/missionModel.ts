import { flowTitle, formatRunDate, runStatusKind, type RunStatusKind } from '@/features/runs/model/runOverviewModel'
import { formatDuration, type RunRecord } from '@/features/runs/model/shared'
import type { Mission, RosterEntry } from '../MissionsPanel'

// What the Missions tab says about a mission: its one short line, the events
// its conversation's user turns carry, and its runs joined with their facts.

const TERMINAL = ['completed', 'failed', 'canceled', 'validation_error']
const closedWords = { done: 'Done', failed: 'Failed', canceled: 'Canceled' } as const

/** Mission timestamps are UTC `YYYY-MM-DD H:MM:SS.fraction +00:00:00` (hour unpadded); other values pass through. */
export function missionTime(value: string | null | undefined): string {
    const match = /^(\d{4}-\d{2}-\d{2}) (\d{1,2})(:\d{2}:\d{2})/.exec(value ?? '')
    return match ? `${match[1]}T${match[2].padStart(2, '0')}${match[3]}Z` : value ?? ''
}

export const formatMissionDate = (value: string | null | undefined, now = Date.now()) => formatRunDate(missionTime(value), now)

/** The question a mission that needs you is waiting on, as the run asked it. */
export type MissionQuestion = { prompt: string; options: string[]; runId: string | null; flowName: string }

/** Options arrive as `{label, value}` objects or plain strings. */
export function readQuestion(payload: unknown): MissionQuestion | null {
    if (!payload || typeof payload !== 'object') return null
    const value = payload as Record<string, unknown>
    const text = (key: string) => typeof value[key] === 'string' ? value[key] as string : ''
    const options = Array.isArray(value.options) ? value.options.map(option => typeof option === 'string' ? option : String((option as Record<string, unknown>)?.label ?? (option as Record<string, unknown>)?.value ?? '')).filter(Boolean) : []
    return { prompt: text('prompt'), options, runId: text('root_run_id') || text('run_id') || null, flowName: text('flow_name') }
}

/** One short line saying where the mission is; a closed mission's reason never shows here. */
export function shortLine(mission: Mission, now = Date.now()): string {
    switch (mission.status ?? 'draft') {
        case 'draft': return `Draft · created ${formatMissionDate(mission.created_at ?? mission.updated_at, now)}`
        case 'needs_you': return readQuestion(mission.question)?.prompt || 'Waiting for your reply'
        case 'closed': return `${closedWords[mission.closed?.status ?? 'done']} · ${formatMissionDate(mission.closed?.at, now)}`
        case 'running': {
            if (mission.waiting) return `Waiting: ${mission.wait_reason}`
            const run = mission.runs?.find(entry => !TERMINAL.includes(entry.status))
            return run ? `${run.summary || flowTitle(run.flow_name)} · ${formatDuration(missionTime(run.launched_at), null, 'running', now)}` : 'Agent is working'
        }
    }
}

export type TurnEvent =
    | { kind: 'start' }
    | { kind: 'result'; runId: string; flowName: string; summary: string; status: string; error: string }
    | ({ kind: 'question' } & MissionQuestion)
    | { kind: 'reply'; text: string }
    | { kind: 'text'; text: string }

// The runtime joins a turn's events with blank lines; split only where the next event starts.
const EVENT_START = /\n\n(?=Run \S+ \(|Run question: |User: |Trigger fired: |The user updated this mission)/
const RESULT = /^Run (\S+) \(([^,]*), ("(?:[^"\\]|\\.)*")\) (?:ended (\w*)|is waiting on a recovery decision)(?:: ([\s\S]*))?\.$/

/** Reads the events a mission's user turn carries: its start, run results, run questions, and your replies. */
export function parseTurn(content: string): TurnEvent[] {
    return content.split(EVENT_START).map((chunk): TurnEvent => {
        if (chunk === 'Begin work on this mission.') return { kind: 'start' }
        if (chunk.startsWith('User: ')) return { kind: 'reply', text: chunk.slice(6) }
        if (chunk.startsWith('Run question: ')) {
            try {
                const question = readQuestion(JSON.parse(chunk.slice(14)))
                if (question) return { kind: 'question', ...question }
            } catch { /* not JSON: show it as written */ }
        }
        const result = RESULT.exec(chunk)
        if (result) {
            let summary = ''
            try { summary = JSON.parse(result[3]) } catch { summary = result[3].slice(1, -1) }
            return { kind: 'result', runId: result[1], flowName: result[2], summary, status: result[4] ?? 'waiting', error: result[5] ?? '' }
        }
        return { kind: 'text', text: chunk }
    })
}

export type MissionRun = {
    runId: string; title: string; flowTitle: string; status: string; kind: RunStatusKind
    duration: string; tokens: number | null
}

/** Each roster entry with the facts the run record holds: its title, live status, duration and tokens. */
export function joinRuns(roster: RosterEntry[], runs: RunRecord[], now = Date.now()): MissionRun[] {
    const byId = new Map(runs.map(run => [run.run_id, run]))
    return roster.map(entry => {
        const run = byId.get(entry.run_id)
        // The roster knows about human gates; the run record's own status is fresher otherwise.
        const status = entry.status === 'waiting' ? 'waiting' : run?.status ?? entry.status
        const duration = run ? formatDuration(run.started_at, run.ended_at, run.status, now) : formatDuration(missionTime(entry.launched_at), null, status, now)
        return {
            runId: entry.run_id,
            title: run?.title?.trim() || entry.summary || flowTitle(entry.flow_name),
            flowTitle: flowTitle(run?.flow_name ?? entry.flow_name),
            status,
            kind: runStatusKind(status, false),
            duration: duration === '—' ? '' : duration,
            tokens: run?.token_usage ?? null,
        }
    })
}

/** Tokens spent across the mission's runs, when any run reports them. */
export function totalTokens(runs: MissionRun[]): number | null {
    const known = runs.filter(run => run.tokens !== null)
    return known.length ? known.reduce((sum, run) => sum + (run.tokens ?? 0), 0) : null
}

export const runMarks: Record<RunStatusKind, { mark: string; tone: string; label: string }> = {
    completed: { mark: '✓', tone: 'text-success', label: 'Completed' },
    failed: { mark: '✕', tone: 'text-destructive', label: 'Failed' },
    waiting: { mark: '?', tone: 'text-warning', label: 'Waiting' },
    running: { mark: '•', tone: 'text-primary', label: 'Running' },
    ended: { mark: '–', tone: 'text-muted-foreground', label: 'Ended' },
}
