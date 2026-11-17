import type { RunRecord } from './shared'
import { humanizeNodeId, isSystemContextKey, parseBranchFailure, type RunVisit } from './visitModel'

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

/** A run's flow as the flow titles itself when it launched, else as its file name reads. */
export const runFlowTitle = (run: Pick<RunRecord, 'flow_name' | 'flow_title'>): string => (
    run.flow_title?.trim() || flowTitle(run.flow_name)
)

// Shared formatters: building one per call dominates rendering a long run list.
const TIME_FORMAT = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
const MONTH_DAY_FORMAT = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })
const MONTH_DAY_YEAR_FORMAT = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' })

/** "14:05" today, "Sep 27" this year, "Sep 27, 2025" before. */
export function formatRunDate(timestamp: string | null | undefined, now: number): string {
    const time = timestamp ? Date.parse(timestamp) : Number.NaN
    if (!Number.isFinite(time)) {
        return ''
    }
    const date = new Date(time)
    const today = new Date(now)
    if (date.toDateString() === today.toDateString()) {
        return TIME_FORMAT.format(date)
    }
    return (date.getFullYear() === today.getFullYear() ? MONTH_DAY_FORMAT : MONTH_DAY_YEAR_FORMAT).format(date)
}

const TIME_SECONDS_FORMAT = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit', second: '2-digit' })

/** "14:05": the start time of a run listed under a day group, which already names the day; "14:05:09" with `seconds`. */
export function formatRunTime(timestamp: string | null | undefined, seconds = false): string {
    const time = timestamp ? Date.parse(timestamp) : Number.NaN
    return Number.isFinite(time) ? (seconds ? TIME_SECONDS_FORMAT : TIME_FORMAT).format(time) : ''
}

/** The day group a run lists under: "Today", "Yesterday", then "Sep 27"-style dates; "" without a date. */
export function runDayLabel(timestamp: string | null | undefined, now: number): string {
    const time = timestamp ? Date.parse(timestamp) : Number.NaN
    if (!Number.isFinite(time)) {
        return ''
    }
    const day = new Date(time).toDateString()
    if (day === new Date(now).toDateString()) {
        return 'Today'
    }
    if (day === new Date(now - 24 * 60 * 60 * 1000).toDateString()) {
        return 'Yesterday'
    }
    return formatRunDate(timestamp, now)
}

/** "5.5M", "193k", "812". */
export function formatCompactCount(value: number): string {
    return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(value).replace('K', 'k')
}

/** The trigger that started a run, from the launch context Spark gives every run a trigger starts. */
export function runTrigger(run: Pick<RunRecord, 'launch_context'>): { id: string; name: string } | null {
    const value = run.launch_context?.['context.spark_trigger']
    if (!value || typeof value !== 'object') return null
    const { trigger_id: id, trigger_name: name } = value as Record<string, unknown>
    return typeof id === 'string' && id ? { id, name: typeof name === 'string' && name ? name : id } : null
}

/** The run's title, else the first launch input it was given in its flow's order, else its flow title. */
export const runTitle = (run: Pick<RunRecord, 'title' | 'flow_name' | 'flow_title' | 'first_launch_input'>): string => (
    run.title?.trim() || run.first_launch_input?.trim() || runFlowTitle(run)
)

// The runtime's error text is the only record of what failed: a backend,
// container or usage-limit failure means the infrastructure failed, not the work.
const INFRASTRUCTURE_ERROR = /codergen backend failed|app-server|usage limit|rate limit|execution container|container node worker|daemon|docker/i

export type RunStatusKind = 'waiting' | 'failed' | 'completed' | 'running' | 'ended'

const ACTIVE_STATUSES = new Set(['running', 'queued', 'pause_requested', 'abort_requested', 'cancel_requested'])

/**
 * Which story the status item tells: a pending question wins over everything else. A run that
 * completed at a failure exit (`outcome: 'failure'`) did not succeed, so lists count it as failed.
 */
export function runStatusKind(status: string, hasPendingQuestion: boolean, outcome?: string | null): RunStatusKind {
    if (hasPendingQuestion || status === 'waiting') return 'waiting'
    if (status === 'failed' || status === 'validation_error' || status === 'unreadable') return 'failed'
    if (status === 'completed') return outcome === 'failure' ? 'failed' : 'completed'
    return ACTIVE_STATUSES.has(status) ? 'running' : 'ended'
}

/** A listed run's state, counting a failure exit as failed. */
export const runKind = (run: Pick<RunRecord, 'status' | 'outcome'>): RunStatusKind => runStatusKind(run.status, false, run.outcome)

const STATUS_SEARCH_WORDS: Record<RunStatusKind, string> = {
    waiting: 'needs input waiting',
    running: 'running queued',
    failed: 'failed',
    completed: 'completed',
    ended: 'canceled cancelled stopped',
}

/** Whether a run list search (lower-cased, trimmed) finds a run by its title, its flow or its state. */
export function runMatchesSearch(run: RunRecord, query: string): boolean {
    return !query
        || runTitle(run).toLowerCase().includes(query)
        || runFlowTitle(run).toLowerCase().includes(query)
        || flowTitle(run.flow_name).toLowerCase().includes(query)
        || run.flow_name.toLowerCase().includes(query)
        // Word starts only, so "fail" finds failed runs but "in" doesn't find every waiting one.
        || ` ${STATUS_SEARCH_WORDS[runKind(run)]}`.includes(` ${query}`)
}

/**
 * Whether a search would leave a run out of the run list. The list keeps a run that matches, one with
 * a matching descendant (on the path to it), and every child of a matching parent.
 */
export function searchHidesRun(runs: RunRecord[], runId: string, query: string): boolean {
    const run = runs.find((candidate) => candidate.run_id === runId)
    if (!query || !run) return false
    const parent = run.parent_run_id ? runs.find((candidate) => candidate.run_id === run.parent_run_id) : undefined
    if (runMatchesSearch(run, query) || (parent && runMatchesSearch(parent, query))) return false
    const seen = new Set([run.run_id])
    const below = [run.run_id]
    while (below.length > 0) {
        const id = below.pop()
        for (const child of runs) {
            if (child.parent_run_id !== id || seen.has(child.run_id)) continue
            if (runMatchesSearch(child, query)) return false
            seen.add(child.run_id)
            below.push(child.run_id)
        }
    }
    return true
}

export type RunFailureKind = 'infrastructure' | 'flow'

export function runFailureMessage(run: Pick<RunRecord, 'last_error' | 'outcome_reason_message'>): string {
    return run.last_error?.trim() || run.outcome_reason_message?.trim() || 'The run failed without recording a reason.'
}

export function classifyRunFailure(run: Pick<RunRecord, 'last_error' | 'outcome_reason_message'>): RunFailureKind {
    return INFRASTRUCTURE_ERROR.test(run.last_error ?? '') ? 'infrastructure' : 'flow'
}

// The runtime's own failure texts, said plainly. Anything else is the flow's,
// the tool's or the agent's own words and reads as written.
const FAILURE_WORDINGS: Array<[RegExp, (match: RegExpMatchArray) => string]> = [
    [/^tool exited with (?:unmapped )?code (-?\d+)/, ([, code]) => `The command failed (exit code ${code}).`],
    [/^tool command timed out/, () => 'The command timed out.'],
    [/^tool command could not be launched/, () => 'The command could not start.'],
    [/^tool command stopped by cancellation/, () => 'The command was stopped by a cancel.'],
    [/^tool pre-hook blocked execution/, () => 'The hook that runs before the command blocked it.'],
    [/^tool (?:JSON|stdout|returned undeclared)/, () => 'The command’s output did not name a decision this step can take.'],
    [/^codergen backend failed:[\s\S]*usage limit/i, () => 'The agent could not run: its usage limit was reached.'],
    [/^codergen backend failed:[\s\S]*rate limit/i, () => 'The agent could not run: its provider is limiting requests.'],
    [/^codergen backend failed:[\s\S]*timed out/i, () => 'The agent could not run: it stopped responding.'],
    [/^codergen backend failed/i, () => 'The agent could not run.'],
    [/^visit_limit_reached/, () => 'A step ran more times than the flow allows.'],
    [/^no_matching_route/, () => 'The step’s decision matched none of its routes.'],
    [/^stage_failed$/, () => 'The step failed without giving a reason.'],
]

/** Whether a run's failure reason is the runtime's record that a user canceled it. */
export const isCancelReason = (reason: unknown): boolean => reason === 'aborted_by_user'

/**
 * A failure reason in plain words, with the runtime's own text as the detail when it was reworded:
 * a command's stderr alone when it has one. A fork's error names its branch's step by `nodeLabel`.
 */
export function describeFailure(
    reason: string,
    nodeLabel: (nodeId: string) => string = humanizeNodeId,
): { summary: string; detail: string | null } {
    const text = reason.trim()
    const branch = parseBranchFailure(text)
    if (branch) {
        const cause = describeFailure(branch.reason, nodeLabel)
        const where = branch.node === branch.branch ? '' : ` at ${nodeLabel(branch.node)}`
        return { summary: `The ${nodeLabel(branch.branch)} branch failed${where}. ${cause.summary}`, detail: cause.detail }
    }
    for (const [pattern, words] of FAILURE_WORDINGS) {
        const match = text.match(pattern)
        if (match) {
            return { summary: words(match), detail: text.match(/^tool [^;]*; stderr: ([\s\S]+)$/)?.[1] ?? text }
        }
    }
    return { summary: text, detail: null }
}

/** Visits that ended in an error the run continued past, in run order. */
export const errorVisits = (visits: RunVisit[]): RunVisit[] => (
    visits.filter((visit) => visit.outcome === 'error')
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
