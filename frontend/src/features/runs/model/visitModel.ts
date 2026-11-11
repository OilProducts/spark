import type {
    ChildRunActivityResponse,
    NodeExecutionResponse,
    RunExecutionPrompt,
    RunTranscriptSegment,
} from '@/lib/api/attractorApi'
import { formatElapsed, type TimelineEventEntry } from './shared'
import { segmentTimelineEntry } from '@/features/projects/model/conversationTimeline'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { timestampMs } from './transcriptModel'

// A visit is one execution of one node: run id, node id, stage_index and
// attempt. Visits are derived from the journal (timing, routing, child runs,
// parallel branches) and the executions list (each execution's status.json),
// labelled from the run's flow snapshot.

// A node completes with a decision or ends in error; 'failed' is the error
// the run failed on, 'error' one the run continued past.
export type VisitOutcome =
    | 'completed'
    | 'error'
    | 'failed'
    | 'interrupted'
    | 'running'
    | 'waiting'

export const VISIT_OUTCOME_LABELS: Record<VisitOutcome, string> = {
    completed: 'Completed',
    error: 'Error',
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
    /** The decision the node returned, which selected its route. */
    decision: string | null
    /** An error's kind (runtime, contract, routing) and whether the flow tolerates it. */
    failureKind: string | null
    tolerable: boolean | null
    reason: string | null
    status: Record<string, unknown> | null
    next: { key: string; loopBack: boolean } | null
    childRun: VisitChildRun | null
    /** The fan-out visit a parallel branch visit belongs to. */
    parentKey: string | null
    /**
     * The execution a parallel branch visit recorded, from its journal events. Null
     * when none is known to be its own: its branch reported no node yet, or
     * concurrent branches share the identity. Root visits use their stage and attempt.
     */
    executionIdentity?: { stageIndex: number; attempt: number } | null
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

export const humanizeNodeId = (nodeId: string): string => (
    nodeId.replace(/[_-]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
)

interface VisitDraft {
    visit: RunVisit
    startSequence: number
    terminal: 'completed' | 'error' | null
    journalReason: string | null
    journalDecision: string | null
}

interface BranchDraft extends VisitDraft {
    parent: VisitDraft
    branchIndex: number
}

interface BranchInvocation {
    nodeId: string
    stageIndex: number
    attempt: number
}

// The node invocation a branch node event names.
const invocationOf = ({ payload }: TimelineEventEntry): BranchInvocation | null => {
    const nodeId = asString(payload.branch_node_id)
    const { stage_index: stageIndex, attempt } = payload
    return nodeId && typeof stageIndex === 'number' && typeof attempt === 'number'
        ? { nodeId, stageIndex, attempt }
        : null
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
    const nodeFields = (nodeId: string) => ({
        nodeId,
        label: flowNodes[nodeId]?.label || humanizeNodeId(nodeId),
        kind: flowNodes[nodeId]?.kind ?? 'agent_task',
    })
    const drafts = new Map<string, VisitDraft>()
    const draftFor = (nodeId: string, stageIndex: number, attempt: number, startSequence: number): VisitDraft => {
        const key = visitKey(runId, nodeId, stageIndex, attempt)
        let draft = drafts.get(key)
        if (!draft) {
            draft = {
                visit: {
                    ...nodeFields(nodeId),
                    key,
                    runId,
                    stageIndex,
                    attempt,
                    number: 1,
                    count: 1,
                    startedAt: null,
                    endedAt: null,
                    outcome: 'running',
                    decision: null,
                    failureKind: null,
                    tolerable: null,
                    reason: null,
                    status: null,
                    next: null,
                    childRun: null,
                    parentKey: null,
                },
                startSequence,
                terminal: null,
                journalReason: null,
                journalDecision: null,
            }
            drafts.set(key, draft)
        }
        return draft
    }

    // Stage events: a retry of the same stage starts a new attempt.
    const startsByStage = new Map<string, number>()
    const openByStage = new Map<string, VisitDraft>()
    const branchDrafts: BranchDraft[] = []
    for (const event of events) {
        // An explicit retry is a new attempt, even of a stage a crash left open.
        if (event.type === 'PipelineRetryStarted') {
            openByStage.clear()
        }
        const nodeId = event.nodeId
        const stageIndex = event.stageIndex ?? (typeof event.payload.index === 'number' ? event.payload.index : null)
        if ((event.type === 'StageStarted' || event.type === 'StageCompleted' || event.type === 'StageFailed')
            && nodeId && stageIndex !== null) {
            const stageKey = JSON.stringify([nodeId, stageIndex])
            if (event.type === 'StageStarted') {
                // A resume starts the stage it interrupted again: the same attempt and visit.
                if (openByStage.has(stageKey)) {
                    continue
                }
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
                draft.terminal = 'completed'
                draft.journalReason = asString(event.payload.notes)
                draft.journalDecision = asString(event.payload.decision)
            } else {
                draft.terminal = 'error'
                const error = asString(event.payload.error)
                draft.journalReason = error === 'stage_failed' ? null : error
            }
        }
    }

    const byStageAndAttempt = (left: VisitDraft, right: VisitDraft) => (
        left.visit.stageIndex - right.visit.stageIndex || left.visit.attempt - right.visit.attempt
    )
    // Only journal visits can have started child runs and branches.
    const journalRoots = [...drafts.values()].sort(byStageAndAttempt)
    const latestRootVisitBefore = (nodeId: string | null, sequence: number): VisitDraft | null => {
        let match: VisitDraft | null = null
        for (const draft of journalRoots) {
            if (draft.visit.nodeId === nodeId && draft.startSequence <= sequence) {
                match = draft
            }
        }
        return match
    }

    const branchDraft = (parent: VisitDraft, key: string, nodeId: string, branchIndex: number, event: TimelineEventEntry): BranchDraft => ({
        visit: {
            ...parent.visit,
            ...nodeFields(nodeId),
            key,
            startedAt: event.receivedAt,
            endedAt: null,
            status: null,
            next: null,
            childRun: null,
            parentKey: parent.visit.key,
            executionIdentity: null,
        },
        startSequence: event.sequence,
        terminal: null,
        journalReason: null,
        journalDecision: null,
        parent,
        branchIndex,
    })
    // Each branch's visits, oldest first, by fan-out visit and branch index: a
    // relaunch continues them; another visit of the fan-out has its own.
    const branchVisits = new Map<string, BranchDraft[]>()
    const branchVisitsKey = (parent: VisitDraft, branchIndex: unknown) => JSON.stringify([parent.visit.key, branchIndex])
    const branchVisitsOf = (event: TimelineEventEntry): BranchDraft[] | undefined => {
        const parent = latestRootVisitBefore(event.nodeId, event.sequence)
        return parent ? branchVisits.get(branchVisitsKey(parent, event.payload.index)) : undefined
    }
    const identifyBranchDraft = ({ visit }: BranchDraft, { nodeId, stageIndex, attempt }: BranchInvocation) => {
        Object.assign(visit, nodeFields(nodeId), { stageIndex, attempt, executionIdentity: { stageIndex, attempt } })
    }
    const isInvocation = ({ visit }: BranchDraft, { nodeId, stageIndex, attempt }: BranchInvocation) => (
        Boolean(visit.executionIdentity) && visit.nodeId === nodeId && visit.stageIndex === stageIndex && visit.attempt === attempt
    )
    const closeBranchDraft = (draft: BranchDraft, event: TimelineEventEntry, terminal: NonNullable<VisitDraft['terminal']>) => {
        draft.visit.endedAt = event.receivedAt
        draft.terminal = terminal
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
            const parent = journalRoots.find((draft) => draft.visit.childRun?.runId === childRunId)
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
                // A relaunch after a resume changes nothing about the branch so far.
                if (!branchVisits.has(branchVisitsKey(parent, branchIndex))) {
                    const key = JSON.stringify([runId, branch, parent.visit.stageIndex, parent.visit.attempt, 'branch', branchIndex])
                    const draft = branchDraft(parent, key, branch, branchIndex, event)
                    branchDrafts.push(draft)
                    branchVisits.set(branchVisitsKey(parent, branchIndex), [draft])
                }
            }
        } else if (event.type === 'ParallelBranchNodeStarted') {
            // Each node invocation a branch runs is a visit of its own. The first confirms
            // the visit the branch's start made; a replay names one the branch already has.
            const invocation = invocationOf(event)
            const visits = branchVisitsOf(event)
            const latest = visits?.at(-1)
            if (invocation && visits && latest) {
                if (!latest.visit.executionIdentity) {
                    identifyBranchDraft(latest, invocation)
                } else if (!visits.some((draft) => isInvocation(draft, invocation))) {
                    if (latest.terminal === null) {
                        closeBranchDraft(latest, event, 'completed')
                    }
                    const { nodeId, stageIndex, attempt } = invocation
                    const key = JSON.stringify([latest.parent.visit.key, 'branch', latest.branchIndex, nodeId, stageIndex, attempt])
                    const draft = branchDraft(latest.parent, key, nodeId, latest.branchIndex, event)
                    identifyBranchDraft(draft, invocation)
                    branchDrafts.push(draft)
                    visits.push(draft)
                }
            }
        } else if (event.type === 'ParallelBranchNodeCompleted') {
            const invocation = invocationOf(event)
            const draft = invocation && branchVisitsOf(event)?.find((candidate) => isInvocation(candidate, invocation))
            if (draft && draft.terminal === null) {
                closeBranchDraft(draft, event, event.payload.outcome === 'error' ? 'error' : 'completed')
            }
        } else if (event.type === 'ParallelBranchCompleted') {
            const latest = branchVisitsOf(event)?.at(-1)
            if (latest?.terminal === null) {
                closeBranchDraft(latest, event, event.payload.success === false ? 'error' : 'completed')
            }
        }
    }

    // ponytail: concurrent branches on one node share an execution under the existing storage
    // scheme, so those visits stay journal-only; give each branch its own record to lift this.
    const branchesByExecution = new Map<string, BranchDraft[]>()
    for (const draft of branchDrafts) {
        if (draft.visit.executionIdentity) {
            const key = visitKey(runId, draft.visit.nodeId, draft.visit.stageIndex, draft.visit.attempt)
            branchesByExecution.set(key, [...(branchesByExecution.get(key) ?? []), draft])
        }
    }
    for (const sharing of branchesByExecution.values()) {
        if (sharing.length > 1) {
            sharing.forEach((draft) => { draft.visit.executionIdentity = null })
        }
    }

    for (const execution of executions) {
        if (execution.run_id !== runId) {
            continue
        }
        const key = visitKey(runId, execution.node_id, execution.stage_index, execution.attempt)
        const branches = branchesByExecution.get(key)
        if (branches?.length === 1) {
            branches[0].visit.status = execution.status
            continue
        }
        // An execution the journal never started is a visit only when its node is
        // in the flow; runtime-internal executions (the post-run result summary)
        // are not visits.
        if (!drafts.has(key) && !flowNodes[execution.node_id]) {
            continue
        }
        const draft = draftFor(execution.node_id, execution.stage_index, execution.attempt, Number.POSITIVE_INFINITY)
        draft.visit.status = execution.status
    }
    const rootDrafts = [...drafts.values()].sort(byStageAndAttempt)

    const runEnded = runStatus !== null && TERMINAL_RUN_STATUSES.has(runStatus)
    // A node stopped by cancel has no stage end of its own; it ends with the run.
    const runEndedAt = [...events].reverse()
        .find((event) => event.type === 'PipelineFailed' || event.type === 'PipelineCompleted')?.receivedAt ?? null
    const waiting = new Set(waitingNodeIds)
    const settle = (draft: VisitDraft, isLastRootVisit: boolean) => {
        const { visit } = draft
        const status = visit.status
        const statusValue = asString(status?.status)
        const terminal = statusValue === 'error' || statusValue === 'completed' ? statusValue : draft.terminal
        if (terminal === 'error') {
            visit.outcome = isLastRootVisit && runStatus === 'failed' ? 'failed' : 'error'
        } else if (terminal) {
            visit.outcome = terminal
        } else if (runEnded) {
            visit.outcome = 'interrupted'
            visit.endedAt ??= runEndedAt
        } else {
            visit.outcome = waiting.has(visit.nodeId) ? 'waiting' : 'running'
        }
        // A subflow visit's notes are the runtime's "Child completed"; its child
        // run summary says what happened, so only a failure reason is kept.
        visit.decision = asString(status?.decision) ?? draft.journalDecision
        visit.failureKind = asString(status?.failure_kind)
        visit.tolerable = typeof status?.tolerable === 'boolean' ? status.tolerable : null
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
        ordered.push(...branchDrafts
            .filter((branch) => branch.parent === draft)
            .sort((left, right) => left.branchIndex - right.branchIndex || left.startSequence - right.startSequence)
            .map((branch) => branch.visit))
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
    return durationMs === null ? '' : formatElapsed(durationMs)
}

/** An error's kind and whether the flow tolerates it, such as "runtime error, tolerable". */
export function formatVisitErrorKind(visit: Pick<RunVisit, 'failureKind' | 'tolerable'>): string | null {
    const parts = [
        visit.failureKind ? `${visit.failureKind} error` : null,
        visit.tolerable === true ? 'tolerable' : visit.tolerable === false ? 'not tolerable' : null,
    ].filter(Boolean)
    return parts.length > 0 ? parts.join(', ') : null
}

export type VisitMark = 'error' | 'failed' | 'interrupted' | 'waiting' | 'loop_back'

/** The one mark a visit row shows, most urgent first; only notable visits get one. */
export function visitMark(visit: RunVisit): VisitMark | null {
    if (visit.outcome === 'failed' || visit.outcome === 'waiting'
        || visit.outcome === 'interrupted' || visit.outcome === 'error') {
        return visit.outcome
    }
    return visit.next?.loopBack ? 'loop_back' : null
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
    visit.executionIdentity !== null
    && (item.source_run_id ?? item.run_id) === visit.runId
    && item.node_id === visit.nodeId
    && item.stage_index === visit.stageIndex
    && item.attempt === visit.attempt
)

export function visitPrompt(visit: RunVisit, prompts: RunExecutionPrompt[]): string | null {
    return prompts.find((prompt) => isVisitSegment(visit, prompt))?.content ?? null
}

// The final structured response is shown under Returned, so the transcript
// drops an assistant message that is only the decision envelope.
const isStatusEnvelope = (content: string): boolean => {
    const trimmed = content.trim().replace(/^```(?:json)?\s*/, '').replace(/\s*```$/, '')
    if (!trimmed.startsWith('{')) {
        return false
    }
    try {
        const parsed = JSON.parse(trimmed)
        return Boolean(parsed) && typeof parsed === 'object' && 'decision' in parsed
    } catch {
        return false
    }
}

/** One declared branch's settled result, from a fan-out's `parallel.results`. */
export interface ParallelBranchResult {
    id: string
    index: number
    status: string
    decisions: Array<{ node: string; decision: string; target: string | null }>
    notes: string | null
    error: { node: string | null; kind: string | null; tolerable: boolean | null; reason: string | null; attempts: number | null } | null
    terminal: { exitId: string | null; name: string | null; outcome: string | null; node: string | null } | null
}

const asRecord = (value: unknown): Record<string, unknown> | null => (
    value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
)

/** The branch results a fan-out visit wrote, in declared order; empty for any other visit. */
export function parallelBranchResults(visit: Pick<RunVisit, 'status'>): ParallelBranchResult[] {
    const results = asRecord(visit.status?.context_updates)?.['parallel.results']
    if (!Array.isArray(results)) {
        return []
    }
    return results.flatMap((entry, position) => {
        const record = asRecord(entry)
        if (!record) {
            return []
        }
        const error = asRecord(record.error)
        const terminal = asRecord(record.terminal)
        const result = asRecord(terminal?.result)
        const decisions = Array.isArray(record.decisions) ? record.decisions : []
        return [{
            id: asString(record.id) ?? `branch ${position + 1}`,
            index: typeof record.index === 'number' ? record.index : position,
            status: asString(record.status) ?? 'unknown',
            decisions: decisions.flatMap((item) => {
                const decision = asRecord(item)
                const node = asString(decision?.node)
                const chosen = asString(decision?.decision)
                return node && chosen ? [{ node, decision: chosen, target: asString(decision?.target) }] : []
            }),
            notes: asString(record.notes),
            error: error ? {
                node: asString(error.node),
                kind: asString(error.kind),
                tolerable: typeof error.tolerable === 'boolean' ? error.tolerable : null,
                reason: asString(error.reason),
                attempts: typeof error.attempts === 'number' ? error.attempts : null,
            } : null,
            terminal: terminal ? {
                exitId: asString(terminal.exit_id),
                name: asString(result?.name),
                outcome: asString(result?.outcome),
                node: asString(terminal.node),
            } : null,
        }]
    })
}

/**
 * One visit's segments as transcript entries. Questions, flow requests, launches and run
 * recoveries are left to the raw journal, so the transcript stays on the agent exchange; ids stay segment ids, unique within
 * the visit's one execution.
 */
export function visitTranscriptEntries(visit: RunVisit, segments: RunTranscriptSegment[]): ConversationTimelineEntry[] {
    return segments
        .filter((segment) => isVisitSegment(visit, segment))
        .sort((left, right) => left.order - right.order)
        .flatMap((segment) => {
            const entry = segmentTimelineEntry(segment)
            if (
                !entry
                || entry.kind === 'request_user_input'
                || entry.kind === 'flow_run_request'
                || entry.kind === 'flow_launch'
                || entry.kind === 'run_recovery'
                || ((entry.kind === 'message' || entry.kind === 'plan') && isStatusEnvelope(entry.content))
            ) {
                return []
            }
            return [entry]
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
