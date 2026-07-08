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
    turnId: string
    nodeId: string | null
    attempt: number
    sourceScope: 'root' | 'child'
    sourceFlowName: string | null
    latestSequence: number
    rows: RunTranscriptRow[]
}

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
        let group = groups.get(segment.turn_id)
        if (!group) {
            group = {
                turnId: segment.turn_id,
                nodeId: segment.node_id,
                attempt: segment.attempt,
                sourceScope: segment.source_scope,
                sourceFlowName: segment.source_flow_name,
                latestSequence: segment.latest_sequence,
                rows: [],
            }
            groups.set(segment.turn_id, group)
        }
        group.rows.push(row)
        group.latestSequence = Math.max(group.latestSequence, segment.latest_sequence)
    }
    for (const group of groups.values()) {
        group.rows.sort((left, right) => left.segment.order - right.segment.order)
    }
    return Array.from(groups.values()).sort((left, right) => {
        if (left.nodeId === right.nodeId) {
            return left.attempt - right.attempt
        }
        // Stable run order: groups appear in first-touched journal order.
        return 0
    })
}

export function runTranscriptGroupLabel(group: RunTranscriptGroup): string {
    const node = group.nodeId ?? 'run'
    const child = group.sourceScope === 'child' && group.sourceFlowName
        ? ` (${group.sourceFlowName})`
        : ''
    const attempt = group.attempt > 0 ? ` — attempt ${group.attempt + 1}` : ''
    return `${node}${child}${attempt}`
}
