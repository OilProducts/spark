import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type {
    TranscriptMessageEntry,
    TranscriptToolCallEntry,
} from '@/components/app/transcript/SegmentRows'

// Maps projected run segments onto the shared transcript row shapes. A run
// transcript is the same activity a chat turn is; the visit model scopes rows
// to one execution.

export type RunTranscriptRow =
    | { kind: 'message'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'thinking'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'tool_call'; segment: RunTranscriptSegment; entry: TranscriptToolCallEntry }

// Turn and segment ids are only unique within one node execution, so
// identity is composed from the execution (run, node, visit, attempt).
export const runTranscriptExecutionKey = (segment: RunTranscriptSegment): string => (
    JSON.stringify([segment.source_run_id, segment.node_id, segment.stage_index, segment.attempt])
)

export const runTranscriptSegmentKey = (segment: RunTranscriptSegment): string => (
    `${runTranscriptExecutionKey(segment)}:${segment.id}`
)

export const timestampMs = (value: string | null | undefined): number => Date.parse(value ?? '') || 0

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
    // in the raw journal; the transcript stays focused on the agent exchange.
    return null
}
