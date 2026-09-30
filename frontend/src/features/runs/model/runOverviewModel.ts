import type { RunRecord } from './shared'
import { isSystemContextKey, type RunVisit } from './visitModel'

// What the Runs page says about a run as a whole: its title, how it failed,
// which visits didn't pass, and its context with each key's history.

/** "software-development/implement-change.yaml" reads as "Implement Change". */
export function flowTitle(flowName: string | null | undefined): string {
    const base = (flowName ?? '').split('/').pop()?.replace(/\.(ya?ml|dot|json)$/i, '') ?? ''
    if (!base) {
        return 'Untitled flow'
    }
    // Child runs already record their flow's title.
    return /[A-Z ]/.test(base)
        ? base
        : base.replace(/[_-]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
}

/** "14:05" today, "Sep 27" this year, "Sep 27, 2025" before. */
export function formatRunDate(timestamp: string | null | undefined, now: number): string {
    const time = timestamp ? Date.parse(timestamp) : Number.NaN
    if (!Number.isFinite(time)) {
        return ''
    }
    const date = new Date(time)
    const today = new Date(now)
    if (date.toDateString() === today.toDateString()) {
        return date.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
    }
    return date.toLocaleDateString(undefined, date.getFullYear() === today.getFullYear()
        ? { month: 'short', day: 'numeric' }
        : { month: 'short', day: 'numeric', year: 'numeric' })
}

/** "5.5M", "193k", "812". */
export function formatCompactCount(value: number): string {
    return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(value).replace('K', 'k')
}

// ponytail: inputs come in key order, not the flow's declared order; read the
// flow's launch inputs if the first key often isn't the telling one.
const firstLaunchInput = (run: Pick<RunRecord, 'launch_context' | 'first_launch_input'>): string | null => {
    for (const value of Object.values(run.launch_context ?? {})) {
        const text = typeof value === 'string' ? value.trim() : ''
        if (text) {
            return text.split('\n')[0].slice(0, 160)
        }
    }
    return run.first_launch_input?.trim() || null
}

/** The run's generated title, else the first launch input it was given, else its flow title. */
export function runTitle(run: Pick<RunRecord, 'title' | 'flow_name' | 'launch_context' | 'first_launch_input'>): string {
    const title = run.title?.trim()
    if (title) {
        return title
    }
    const input = firstLaunchInput(run)
    return input ?? flowTitle(run.flow_name)
}

// The runtime's error text is the only record of what failed: a backend,
// container or usage-limit failure means the infrastructure failed, not the work.
const INFRASTRUCTURE_ERROR = /codergen backend failed|app-server|usage limit|rate limit|execution container|container node worker|daemon|docker/i

export type RunStatusKind = 'waiting' | 'failed' | 'completed' | 'running' | 'ended'

const ACTIVE_STATUSES = new Set(['running', 'queued', 'pause_requested', 'abort_requested', 'cancel_requested'])

/** Which story the status item tells: a pending question wins over everything else. */
export function runStatusKind(status: string, hasPendingQuestion: boolean): RunStatusKind {
    if (hasPendingQuestion || status === 'waiting') return 'waiting'
    if (status === 'failed' || status === 'validation_error') return 'failed'
    if (status === 'completed') return 'completed'
    return ACTIVE_STATUSES.has(status) ? 'running' : 'ended'
}

export type RunFailureKind = 'infrastructure' | 'flow'

export function runFailureMessage(run: Pick<RunRecord, 'last_error' | 'outcome_reason_message'>): string {
    return run.last_error?.trim() || run.outcome_reason_message?.trim() || 'The run failed without recording a reason.'
}

export function classifyRunFailure(run: Pick<RunRecord, 'last_error' | 'outcome_reason_message'>): RunFailureKind {
    return INFRASTRUCTURE_ERROR.test(run.last_error ?? '') ? 'infrastructure' : 'flow'
}

/** Visits whose work came back as not passing, in run order. */
export const didNotPassVisits = (visits: RunVisit[]): RunVisit[] => (
    visits.filter((visit) => visit.outcome === 'did_not_pass')
)

export interface ContextHistoryEntry {
    value: unknown
    /** The visit that wrote this value; null for a launch input. */
    visit: RunVisit | null
}

export interface ContextKeyHistory {
    key: string
    /** Final value; null once cleared. */
    value: unknown
    /** Launch input, then each visit's write, in run order. */
    history: ContextHistoryEntry[]
}

export interface ContextNamespace {
    name: string
    keys: ContextKeyHistory[]
}

export interface RunContextOverview {
    namespaces: ContextNamespace[]
    systemKeys: string[]
}

/** The namespace a key belongs to: "context.review.summary" is in "review". */
export const contextNamespace = (key: string): string => (
    key.replace(/^context\./, '').split('.')[0]
)

export function buildRunContextOverview({
    visits,
    launchContext,
    finalContext,
}: {
    visits: RunVisit[]
    launchContext: Record<string, unknown> | null | undefined
    finalContext: Record<string, unknown> | null | undefined
}): RunContextOverview {
    const histories = new Map<string, ContextHistoryEntry[]>()
    const systemKeys = new Set<string>()
    const record = (key: string, entry: ContextHistoryEntry) => {
        if (isSystemContextKey(key)) {
            systemKeys.add(key)
            return
        }
        histories.set(key, [...(histories.get(key) ?? []), entry])
    }
    for (const [key, value] of Object.entries(launchContext ?? {})) {
        record(key, { value, visit: null })
    }
    for (const visit of visits) {
        const updates = visit.status?.context_updates
        if (updates && typeof updates === 'object' && !Array.isArray(updates)) {
            for (const [key, value] of Object.entries(updates)) {
                record(key, { value, visit })
            }
        }
    }
    for (const key of Object.keys(finalContext ?? {})) {
        if (isSystemContextKey(key)) {
            systemKeys.add(key)
        } else if (!histories.has(key)) {
            histories.set(key, [])
        }
    }

    const byNamespace = new Map<string, ContextKeyHistory[]>()
    for (const key of [...histories.keys()].sort()) {
        const history = histories.get(key)!
        // The visits' writes are the live record; the snapshot catches up at the
        // next checkpoint and covers keys no visit wrote.
        // ponytail: a parallel branch's write only reaches the final value through
        // the snapshot; read the fan-in's merge if branches often write shared keys.
        const lastWrite = [...history].reverse().find((entry) => entry.visit !== null && entry.visit.parentKey === null)
        const value = lastWrite
            ? lastWrite.value
            : finalContext && Object.hasOwn(finalContext, key)
                ? finalContext[key]
                : history.at(-1)?.value ?? null
        const namespace = contextNamespace(key)
        byNamespace.set(namespace, [...(byNamespace.get(namespace) ?? []), { key, value, history }])
    }
    return {
        namespaces: [...byNamespace.entries()].map(([name, keys]) => ({ name, keys })),
        systemKeys: [...systemKeys].sort(),
    }
}
