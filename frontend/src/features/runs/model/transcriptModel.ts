import type { RunTranscriptSegment } from '@/lib/api/attractorApi'

// Turn and segment ids are only unique within one node execution, so
// identity is composed from the execution (run, node, visit, attempt).
export const runTranscriptExecutionKey = (segment: RunTranscriptSegment): string => (
    JSON.stringify([segment.source_run_id, segment.node_id, segment.stage_index, segment.attempt])
)

export const runTranscriptSegmentKey = (segment: RunTranscriptSegment): string => (
    `${runTranscriptExecutionKey(segment)}:${segment.id}`
)

export const timestampMs = (value: string | null | undefined): number => Date.parse(value ?? '') || 0
