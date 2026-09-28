import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type {
    TranscriptMessageEntry,
    TranscriptToolCallEntry,
} from '@/components/app/transcript/SegmentRows'

// Maps projected run segments onto the shared transcript row shapes. A run
// transcript is the same activity a chat turn is; only run-specific grouping
// (node, attempt, child-run scope) is layered on top.

export type RunTranscriptRow =
    | { kind: 'message'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'thinking'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'tool_call'; segment: RunTranscriptSegment; entry: TranscriptToolCallEntry }

export interface RunTranscriptGroup {
    key: string
    turnId: string
    runId: string | null
    nodeId: string | null
    stageIndex: number
    attempt: number
    /** 1-based visit among this node's executions; null when the node ran once. */
    visit: number | null
    sourceScope: 'root' | 'child'
    sourceFlowName: string | null
    latestSequence: number
    /** Newest segment time in epoch ms; comparable with journal event times. */
    latestTime: number
    rows: RunTranscriptRow[]
}

// Turn and segment ids are only unique within one node execution, so
// identity is composed from the execution (run, node, visit, attempt).
export const runTranscriptExecutionKey = (segment: RunTranscriptSegment): string => (
    JSON.stringify([segment.source_run_id, segment.node_id, segment.stage_index, segment.attempt])
)

export const runTranscriptSegmentKey = (segment: RunTranscriptSegment): string => (
    `${runTranscriptExecutionKey(segment)}:${segment.id}`
)

export const timestampMs = (value: string | null | undefined): number => Date.parse(value ?? '') || 0

const segmentTime = (segment: RunTranscriptSegment): number => (
    Math.max(timestampMs(segment.timestamp), timestampMs(segment.updated_at))
)

const rowStatus = (status: RunTranscriptSegment['status']): string => (
    status === 'running' ? 'streaming' : status
)

export function buildRunTranscriptRow(segment: RunTranscriptSegment): RunTranscriptRow | null {
    if (segment.kind === 'assistant_message' || segment.kind === 'plan') {
        return {
            kind: 'message',
            segment,
            entry: {
                id: segment.id,
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: rowStatus(segment.status),
                error: segment.error ?? null,
            },
        }
    }
    if (segment.kind === 'reasoning') {
        return {
            kind: 'thinking',
            segment,
            entry: {
                id: segment.id,
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: rowStatus(segment.status),
                presentation: 'thinking',
            },
        }
    }
    if (segment.kind === 'tool_call' && segment.tool_call) {
        return {
            kind: 'tool_call',
            segment,
            entry: {
                id: segment.id,
                timestamp: segment.timestamp,
                toolCall: {
                    id: segment.tool_call.id,
                    kind: segment.tool_call.kind,
                    status: segment.tool_call.status,
                    completionReason: segment.tool_call.completion_reason ?? null,
                    title: segment.tool_call.title,
                    command: segment.tool_call.command ?? null,
                    output: segment.tool_call.output ?? null,
                    outputSize: segment.tool_call.output_size ?? null,
                    outputTruncated: segment.tool_call.output_truncated === true,
                    filePaths: segment.tool_call.file_paths,
                },
            },
        }
    }
    // Other kinds (agent events, request_user_input, compaction) are visible
    // in the Events view; the transcript stays focused on the agent exchange.
    return null
}

export function buildRunTranscriptGroups(
    segments: RunTranscriptSegment[],
    nodeId?: string | null,
): RunTranscriptGroup[] {
    const groups = new Map<string, RunTranscriptGroup>()
    for (const segment of segments) {
        if (nodeId && segment.node_id !== nodeId) {
            continue
        }
        const row = buildRunTranscriptRow(segment)
        if (!row) {
            continue
        }
        const key = `${runTranscriptExecutionKey(segment)}:${segment.turn_id}`
        let group = groups.get(key)
        if (!group) {
            group = {
                key,
                turnId: segment.turn_id,
                runId: segment.source_run_id,
                nodeId: segment.node_id,
                stageIndex: segment.stage_index,
                attempt: segment.attempt,
                visit: null,
                sourceScope: segment.source_scope,
                sourceFlowName: segment.source_flow_name,
                latestSequence: segment.latest_sequence,
                latestTime: segmentTime(segment),
                rows: [],
            }
            groups.set(key, group)
        }
        group.rows.push(row)
        group.latestSequence = Math.max(group.latestSequence, segment.latest_sequence)
        group.latestTime = Math.max(group.latestTime, segmentTime(segment))
    }
    const visitsByNode = new Map<string, Set<number>>()
    const nodeKey = (group: RunTranscriptGroup) => JSON.stringify([group.runId, group.nodeId])
    for (const group of groups.values()) {
        group.rows.sort((left, right) => left.segment.order - right.segment.order)
        const visits = visitsByNode.get(nodeKey(group)) ?? new Set<number>()
        visitsByNode.set(nodeKey(group), visits.add(group.stageIndex))
    }
    for (const group of groups.values()) {
        const visits = [...(visitsByNode.get(nodeKey(group)) ?? [])].sort((left, right) => left - right)
        group.visit = visits.length > 1 ? visits.indexOf(group.stageIndex) + 1 : null
    }
    return Array.from(groups.values()).sort((left, right) => (
        left.latestTime - right.latestTime || left.stageIndex - right.stageIndex || left.attempt - right.attempt
    ))
}

export function runTranscriptGroupLabel(group: RunTranscriptGroup): string {
    const node = group.nodeId ?? 'run'
    const child = group.sourceScope === 'child' && group.sourceFlowName
        ? ` (${group.sourceFlowName})`
        : ''
    const visit = group.visit !== null ? ` — visit ${group.visit}` : ''
    const attempt = group.attempt > 0 ? ` — attempt ${group.attempt + 1}` : ''
    return `${node}${child}${visit}${attempt}`
}
