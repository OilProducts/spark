import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type {
    TranscriptMessageEntry,
    TranscriptToolCallEntry,
} from '@/components/app/transcript/SegmentRows'
import { toTranscriptToolCall } from '@/components/app/transcript/groupTranscript'

// Maps projected run segments onto the shared transcript row shapes. A run
// transcript is the same activity a chat turn is; the visit model scopes rows
// to one execution.

export type RunTranscriptRow =
    | { kind: 'message'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'thinking'; segment: RunTranscriptSegment; entry: TranscriptMessageEntry }
    | { kind: 'tool_call'; segment: RunTranscriptSegment; entry: TranscriptToolCallEntry }
    /** A context compaction or agent warning, shown as an event line. */
    | { kind: 'event'; segment: RunTranscriptSegment; text: string; warning: boolean }

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
    // A message sent into the running turn: sent, or not sent if the turn
    // ended before the agent read it.
    if (segment.kind === 'user_message') {
        return {
            kind: 'message',
            segment,
            entry: {
                id: segment.id,
                role: 'user',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'failed' ? 'failed' : 'delivered',
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
                toolCall: toTranscriptToolCall(segment.tool_call),
            },
        }
    }
    if (segment.kind === 'context_compaction') {
        return { kind: 'event', segment, text: segment.content || 'Context compacted.', warning: false }
    }
    if (segment.kind === 'agent_event' && segment.category === 'warning') {
        return { kind: 'event', segment, text: segment.content, warning: true }
    }
    // Other kinds (other agent events, request_user_input) are visible in the
    // raw journal; the transcript stays focused on the agent exchange.
    return null
}
