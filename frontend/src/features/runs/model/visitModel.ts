import type {
    ChildRunActivityResponse,
    NodeExecutionResponse,
    RunExecutionPrompt,
    RunTranscriptSegment,
} from '@/lib/api/attractorApi'
import type { TimelineEventEntry } from './shared'
import { buildRunTranscriptRow, timestampMs, type RunTranscriptRow } from './transcriptModel'

// A visit is one execution of one node: run id, node id, stage_index and
// attempt. Visits are derived from the journal (timing, routing, child runs,
// parallel branches) and the executions list (each execution's status.json),
// labelled from the run's flow snapshot.

export type VisitOutcome =
    | 'succeeded'
    | 'partly_succeeded'
    | 'did_not_pass'
    | 'failed'
    | 'interrupted'
    | 'running'
    | 'waiting'

export const VISIT_OUTCOME_LABELS: Record<VisitOutcome, string> = {
    succeeded: 'Succeeded',
    partly_succeeded: 'Partly succeeded',
    did_not_pass: "Didn't pass",
    failed: 'Failed',
    interrupted: 'Interrupted',
    running: 'Running',
    waiting: 'Waiting on a question',
}

export interface VisitFlowNode {
    label: string
    kind: string
    readsContext: string[]
    command: string | null
}

export interface VisitChildRun {
    runId: string
    flowName: string | null
    startedAt: string
    endedAt: string | null
    outcome: string | null
    summary: string | null
}

export interface RunVisit {
    key: string
    runId: string
    nodeId: string
    stageIndex: number
    attempt: number
    label: string
    kind: string
    /** 1-based visit number among this node's visits (n of x). */
    number: number
    count: number
    startedAt: string | null
    endedAt: string | null
    outcome: VisitOutcome
    reason: string | null
    status: Record<string, unknown> | null
    next: { key: string; loopBack: boolean } | null
    childRun: VisitChildRun | null
    /** The fan-out visit a parallel branch visit belongs to. */
    parentKey: string | null
}

export interface VisitModelInput {
    runId: string
    runStatus: string | null
    journal: TimelineEventEntry[]
    executions: NodeExecutionResponse[]
    childRuns: ChildRunActivityResponse[]
    flowNodes: Record<string, VisitFlowNode>
    /** Nodes with a pending question. */
    waitingNodeIds: string[]
}

const TERMINAL_RUN_STATUSES = new Set(['completed', 'failed', 'canceled', 'cancelled', 'aborted'])

export const visitKey = (runId: string, nodeId: string, stageIndex: number, attempt: number): string => (
    JSON.stringify([runId, nodeId, stageIndex, attempt])
)

const asString = (value: unknown): string | null => (
    typeof value === 'string' && value.trim().length > 0 ? value : null
)

const humanizeNodeId = (nodeId: string): string => (
    nodeId.replace(/[_-]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
)

interface VisitDraft {
    visit: RunVisit
    startSequence: number
    terminal: 'succeeded' | 'partly_succeeded' | 'failed' | null
    journalReason: string | null
}

export function buildRunVisits({
    runId,
    runStatus,
    journal,
    executions,
    childRuns,
    flowNodes,
    waitingNodeIds,
}: VisitModelInput): RunVisit[] {
    const events = [...journal]
        .filter((event) => event.sourceScope === 'root')
        .sort((left, right) => left.sequence - right.sequence)
    const drafts = new Map<string, VisitDraft>()
    const draftFor = (nodeId: string, stageIndex: number, attempt: number, startSequence: number): VisitDraft => {
        const key = visitKey(runId, nodeId, stageIndex, attempt)
        let draft = drafts.get(key)
        if (!draft) {
            const node = flowNodes[nodeId]
            draft = {
                visit: {
                    key,
                    runId,
                    nodeId,
                    stageIndex,
                    attempt,
                    label: node?.label || humanizeNodeId(nodeId),
                    kind: node?.kind ?? 'agent_task',
                    number: 1,
                    count: 1,
                    startedAt: null,
                    endedAt: null,
                    outcome: 'running',
                    reason: null,
                    status: null,
                    next: null,
                    childRun: null,
                    parentKey: null,
                },
                startSequence,
                terminal: null,
                journalReason: null,
            }
            drafts.set(key, draft)
        }
        return draft
    }

    // Stage events: a retry of the same stage starts a new attempt.
    const startsByStage = new Map<string, number>()
    const openByStage = new Map<string, VisitDraft>()
    const branchDrafts: VisitDraft[] = []
    for (const event of events) {
        const nodeId = event.nodeId
        const stageIndex = event.stageIndex ?? (typeof event.payload.index === 'number' ? event.payload.index : null)
        if ((event.type === 'StageStarted' || event.type === 'StageCompleted' || event.type === 'StageFailed')
            && nodeId && stageIndex !== null) {
            const stageKey = JSON.stringify([nodeId, stageIndex])
            if (event.type === 'StageStarted') {
                const attempt = startsByStage.get(stageKey) ?? 0
                startsByStage.set(stageKey, attempt + 1)
                const draft = draftFor(nodeId, stageIndex, attempt, event.sequence)
                draft.visit.startedAt = event.receivedAt
                openByStage.set(stageKey, draft)
                continue
            }
            const draft = openByStage.get(stageKey) ?? draftFor(nodeId, stageIndex, 0, event.sequence)
            openByStage.delete(stageKey)
            draft.visit.endedAt = event.receivedAt
            if (event.type === 'StageCompleted') {
                draft.terminal = event.payload.outcome === 'partial_success' ? 'partly_succeeded' : 'succeeded'
                draft.journalReason = asString(event.payload.notes)
            } else {
                draft.terminal = 'failed'
                const error = asString(event.payload.error)
                draft.journalReason = error === 'stage_failed' ? null : error
            }
        }
    }

    for (const execution of executions) {
        if (execution.run_id !== runId) {
            continue
        }
        // An execution the journal never started is a visit only when its node is
        // in the flow; runtime-internal executions (the post-run result summary)
        // are not visits.
        const key = visitKey(runId, execution.node_id, execution.stage_index, execution.attempt)
        if (!drafts.has(key) && !flowNodes[execution.node_id]) {
            continue
        }
        const draft = draftFor(execution.node_id, execution.stage_index, execution.attempt, Number.POSITIVE_INFINITY)
        draft.visit.status = execution.status
    }

    const rootDrafts = [...drafts.values()].sort((left, right) => (
        left.visit.stageIndex - right.visit.stageIndex || left.visit.attempt - right.visit.attempt
    ))
    const latestRootVisitBefore = (nodeId: string | null, sequence: number): VisitDraft | null => {
        let match: VisitDraft | null = null
        for (const draft of rootDrafts) {
            if (draft.visit.nodeId === nodeId && draft.startSequence <= sequence) {
                match = draft
            }
        }
        return match
    }

    // Child runs and parallel branches hang off the visit that started them.
    for (const event of events) {
        if (event.type === 'ChildRunStarted') {
            const childRunId = asString(event.payload.child_run_id)
            const parent = latestRootVisitBefore(asString(event.payload.parent_node_id), event.sequence)
            if (childRunId && parent) {
                parent.visit.childRun = {
                    runId: childRunId,
                    flowName: asString(event.payload.child_flow_name),
                    startedAt: event.receivedAt,
                    endedAt: null,
                    outcome: null,
                    summary: childRunSummary(childRuns, childRunId),
                }
            }
        } else if (event.type === 'ChildRunCompleted') {
            const childRunId = asString(event.payload.child_run_id)
            const parent = rootDrafts.find((draft) => draft.visit.childRun?.runId === childRunId)
            if (parent?.visit.childRun) {
                parent.visit.childRun.endedAt = event.receivedAt
                parent.visit.childRun.outcome = asString(event.payload.outcome)
                parent.visit.childRun.summary ??= asString(event.payload.failure_reason)
            }
        } else if (event.type === 'ParallelBranchStarted') {
            const branch = asString(event.payload.branch)
            const parent = latestRootVisitBefore(event.nodeId, event.sequence)
            if (branch && parent) {
                const branchIndex = typeof event.payload.index === 'number' ? event.payload.index : branchDrafts.length
                const node = flowNodes[branch]
                branchDrafts.push({
                    visit: {
                        ...parent.visit,
                        key: JSON.stringify([runId, branch, parent.visit.stageIndex, parent.visit.attempt, 'branch', branchIndex]),
                        nodeId: branch,
                        label: node?.label || humanizeNodeId(branch),
                        kind: node?.kind ?? 'agent_task',
                        startedAt: event.receivedAt,
                        endedAt: null,
                        status: null,
                        next: null,
                        childRun: null,
                        parentKey: parent.visit.key,
                    },
                    startSequence: event.sequence,
                    terminal: null,
                    journalReason: null,
                })
            }
        } else if (event.type === 'ParallelBranchCompleted') {
            const branch = asString(event.payload.branch)
            const draft = branchDrafts.find((candidate) => (
                candidate.visit.nodeId === branch && candidate.terminal === null
                && JSON.parse(candidate.visit.parentKey ?? '[]')[1] === event.nodeId
            ))
            if (draft) {
                draft.visit.endedAt = event.receivedAt
                draft.terminal = event.payload.success === false ? 'failed' : 'succeeded'
            }
        }
    }

    const runEnded = runStatus !== null && TERMINAL_RUN_STATUSES.has(runStatus)
    const waiting = new Set(waitingNodeIds)
    const settle = (draft: VisitDraft, isLastRootVisit: boolean) => {
        const { visit } = draft
        const status = visit.status
        const statusOutcome = asString(status?.outcome)
        const terminal = draft.terminal ?? (
            statusOutcome === 'fail' ? 'failed'
                : statusOutcome === 'partial_success' ? 'partly_succeeded'
                    : statusOutcome ? 'succeeded'
                        : null
        )
        if (terminal === 'failed') {
            visit.outcome = isLastRootVisit && runStatus === 'failed' ? 'failed' : 'did_not_pass'
        } else if (terminal) {
            visit.outcome = terminal
        } else if (runEnded) {
            visit.outcome = 'interrupted'
        } else {
            visit.outcome = waiting.has(visit.nodeId) ? 'waiting' : 'running'
        }
        // A subflow visit's notes are the runtime's "Child completed"; its child
        // run summary says what happened, so only a failure reason is kept.
        visit.reason = asString(status?.failure_reason)
            ?? (visit.childRun ? null : asString(status?.notes) ?? draft.journalReason)
    }
    rootDrafts.forEach((draft, index) => settle(draft, index === rootDrafts.length - 1))
    branchDrafts.forEach((draft) => settle(draft, false))

    // Where each visit went next, and whether that went back to a node that
    // first ran no later than this one: a loop-back, whatever the outcome.
    const firstRun = new Map<string, number>()
    rootDrafts.forEach((draft, index) => {
        if (!firstRun.has(draft.visit.nodeId)) {
            firstRun.set(draft.visit.nodeId, index)
        }
    })
    rootDrafts.forEach((draft, index) => {
        const next = rootDrafts[index + 1]
        if (next) {
            draft.visit.next = {
                key: next.visit.key,
                loopBack: firstRun.get(next.visit.nodeId)! <= firstRun.get(draft.visit.nodeId)!,
            }
        }
    })

    const ordered: RunVisit[] = []
    for (const draft of rootDrafts) {
        ordered.push(draft.visit)
        for (const branch of branchDrafts) {
            if (branch.visit.parentKey === draft.visit.key) {
                ordered.push(branch.visit)
            }
        }
    }
    const byNode = new Map<string, RunVisit[]>()
    for (const visit of ordered) {
        byNode.set(visit.nodeId, [...(byNode.get(visit.nodeId) ?? []), visit])
    }
    for (const visits of byNode.values()) {
        visits.forEach((visit, index) => {
            visit.number = index + 1
            visit.count = visits.length
        })
    }
    return ordered
}

function childRunSummary(childRuns: ChildRunActivityResponse[], childRunId: string): string | null {
    const child = childRuns.find((candidate) => candidate.run_id === childRunId)
    const executions = [...(child?.executions ?? [])].sort((left, right) => right.stage_index - left.stage_index)
    for (const execution of executions) {
        const notes = asString(execution.status?.notes)
        if (notes) {
            return notes
        }
    }
    return null
}

export function visitDurationMs(visit: Pick<RunVisit, 'startedAt' | 'endedAt'>, now: number): number | null {
    if (!visit.startedAt) {
        return null
    }
    const end = visit.endedAt ? timestampMs(visit.endedAt) : now
    return Math.max(0, end - timestampMs(visit.startedAt))
}

export function formatVisitDuration(durationMs: number | null): string {
    if (durationMs === null) {
        return ''
    }
    const seconds = Math.round(durationMs / 1000)
    if (seconds < 60) {
        return `${seconds}s`
    }
    const minutes = Math.floor(seconds / 60)
    if (minutes < 60) {
        return `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`
    }
    return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`
}

export type VisitMark = 'did_not_pass' | 'failed' | 'interrupted' | 'waiting' | 'loop_back'

/** The marks a visit row shows; only notable visits get any. */
export function visitMarks(visit: RunVisit): VisitMark[] {
    const marks: VisitMark[] = []
    if (visit.outcome === 'did_not_pass' || visit.outcome === 'failed'
        || visit.outcome === 'interrupted' || visit.outcome === 'waiting') {
        marks.push(visit.outcome)
    }
    if (visit.next?.loopBack) {
        marks.push('loop_back')
    }
    return marks
}

// System keys are runtime bookkeeping: anything outside the context.
// namespace, plus the context stack.
export const isSystemContextKey = (key: string): boolean => (
    !key.startsWith('context.') || key.startsWith('context.stack.')
)

const TOOL_RESULT_KEYS = new Set(['context.tool.output', 'context.tool.exit_code'])

export function visitContextWrites(visit: RunVisit): { writes: Array<[string, unknown]>; systemCount: number } {
    const updates = visit.status?.context_updates
    const writes: Array<[string, unknown]> = []
    let systemCount = 0
    if (updates && typeof updates === 'object' && !Array.isArray(updates)) {
        for (const [key, value] of Object.entries(updates)) {
            if (isSystemContextKey(key)) {
                systemCount += 1
            } else if (!(visit.kind === 'tool' && TOOL_RESULT_KEYS.has(key))) {
                writes.push([key, value])
            }
        }
    }
    return { writes, systemCount }
}

export function visitToolResult(visit: RunVisit): { output: string | null; exitCode: number | null } {
    const updates = visit.status?.context_updates as Record<string, unknown> | undefined
    const output = updates?.['context.tool.output']
    const exitCode = updates?.['context.tool.exit_code']
    return {
        output: typeof output === 'string' ? output : null,
        exitCode: typeof exitCode === 'number' ? exitCode : null,
    }
}

const isVisitSegment = (visit: RunVisit, item: { source_run_id?: string | null; run_id?: string; node_id: string | null; stage_index: number; attempt: number }) => (
    (item.source_run_id ?? item.run_id) === visit.runId
    && item.node_id === visit.nodeId
    && item.stage_index === visit.stageIndex
    && item.attempt === visit.attempt
)

export function visitPrompt(visit: RunVisit, prompts: RunExecutionPrompt[]): string | null {
    return prompts.find((prompt) => isVisitSegment(visit, prompt))?.content ?? null
}

// The final structured response is shown under Returned, so the transcript
// drops an assistant message that is only the status envelope.
const isStatusEnvelope = (content: string): boolean => {
    const trimmed = content.trim().replace(/^```(?:json)?\s*/, '').replace(/\s*```$/, '')
    if (!trimmed.startsWith('{')) {
        return false
    }
    try {
        const parsed = JSON.parse(trimmed)
        return Boolean(parsed) && typeof parsed === 'object' && 'outcome' in parsed
    } catch {
        return false
    }
}

export function visitTranscriptRows(visit: RunVisit, segments: RunTranscriptSegment[]): RunTranscriptRow[] {
    if (visit.parentKey) {
        return []
    }
    return segments
        .filter((segment) => isVisitSegment(visit, segment))
        .sort((left, right) => left.order - right.order)
        .flatMap((segment) => {
            const row = buildRunTranscriptRow(segment)
            if (!row || (row.kind === 'message' && isStatusEnvelope(row.entry.content))) {
                return []
            }
            return [row]
        })
}

/** Node labels, kinds and declared reads from the run's flow snapshot. */
export function visitFlowNodesFromSnapshot(flow: Record<string, unknown> | null | undefined): Record<string, VisitFlowNode> {
    const nodes = flow?.nodes
    if (!nodes || typeof nodes !== 'object' || Array.isArray(nodes)) {
        return {}
    }
    return Object.fromEntries(Object.entries(nodes as Record<string, Record<string, unknown>>).map(([nodeId, node]) => {
        const config = (node?.config ?? {}) as Record<string, unknown>
        const contracts = (node?.contracts ?? {}) as Record<string, unknown>
        const reads = Array.isArray(contracts.reads_context) ? contracts.reads_context : []
        return [nodeId, {
            label: asString(node?.label) ?? humanizeNodeId(nodeId),
            kind: asString(node?.kind) ?? asString(config.kind) ?? 'agent_task',
            readsContext: reads.filter((key): key is string => typeof key === 'string'),
            command: asString(config.command),
        }]
    }))
}
