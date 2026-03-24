import type {
    ConversationSegmentResponse,
    ConversationSnapshotResponse,
    ConversationTurnResponse,
} from '@/lib/workspaceClient'
import type { ConversationTimelineEntry } from './types'
import type { OptimisticSendState } from './conversationState'

function formatWorkedDuration(elapsedSeconds: number): string {
    if (elapsedSeconds < 60) {
        return `${elapsedSeconds}s`
    }
    if (elapsedSeconds < 3600) {
        const minutes = Math.floor(elapsedSeconds / 60)
        const seconds = elapsedSeconds % 60
        return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`
    }
    const hours = Math.floor(elapsedSeconds / 3600)
    const minutes = Math.floor((elapsedSeconds % 3600) / 60)
    return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`
}

function resolveWorkedElapsedSeconds(
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
    completedTimestamp: string,
): number | null {
    const completedMs = Date.parse(completedTimestamp)
    if (Number.isNaN(completedMs)) {
        return null
    }
    const candidateTimestamps = [turn.timestamp, ...turnSegments.map((segment) => segment.timestamp)]
        .map((value) => Date.parse(value))
        .filter((value) => !Number.isNaN(value) && value <= completedMs)
    if (candidateTimestamps.length === 0) {
        return null
    }
    const startedMs = Math.min(...candidateTimestamps)
    return Math.max(0, Math.round((completedMs - startedMs) / 1000))
}

function buildAssistantTimelineEntries(
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
): ConversationTimelineEntry[] {
    const entries: ConversationTimelineEntry[] = []
    let hadWorkActivity = false
    let insertedFinalSeparator = false
    const sortedSegments = [...turnSegments].sort((left, right) => left.order - right.order)

    sortedSegments.forEach((segment) => {
        if (!insertedFinalSeparator && hadWorkActivity && segment.kind === 'assistant_message') {
            const elapsedSeconds = resolveWorkedElapsedSeconds(turn, turnSegments, segment.timestamp)
            const label = elapsedSeconds === null
                ? 'Worked'
                : `Worked for ${formatWorkedDuration(elapsedSeconds)}`
            entries.push({
                id: `${turn.id}:final-separator:${entries.length}`,
                kind: 'final_separator',
                role: 'system',
                timestamp: segment.timestamp,
                label,
            })
            insertedFinalSeparator = true
        }
        if (segment.kind === 'assistant_message') {
            entries.push({
                id: segment.id,
                kind: 'message',
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'running' ? 'streaming' : segment.status,
                error: segment.error ?? null,
            })
            return
        }
        if (segment.kind === 'reasoning') {
            entries.push({
                id: segment.id,
                kind: 'message',
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'running' ? 'streaming' : segment.status,
                error: segment.error ?? null,
                presentation: 'thinking',
            })
            return
        }
        if (segment.kind === 'tool_call' && segment.tool_call) {
            entries.push({
                id: segment.id,
                kind: 'tool_call',
                role: 'system',
                timestamp: segment.timestamp,
                toolCall: {
                    id: segment.tool_call.id,
                    kind: segment.tool_call.kind,
                    status: segment.tool_call.status,
                    title: segment.tool_call.title,
                    command: segment.tool_call.command ?? null,
                    output: segment.tool_call.output ?? null,
                    filePaths: segment.tool_call.file_paths,
                },
            })
            hadWorkActivity = true
            return
        }
        if (segment.kind === 'spec_edit_proposal' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'spec_edit_proposal',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === 'flow_run_request' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'flow_run_request',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === 'flow_launch' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'flow_launch',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === 'execution_card' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'execution_card',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
        }
    })

    if (entries.length === 0) {
        const presentation = turn.status === 'complete' || turn.status === 'failed' ? 'default' : 'thinking'
        entries.push({
            id: `${turn.id}:${presentation}:placeholder`,
            kind: 'message',
            role: 'assistant',
            content: turn.content,
            timestamp: turn.timestamp,
            status: turn.status,
            error: turn.error ?? null,
            presentation,
        })
    }

    return entries
}

export function buildConversationTimelineEntries(
    snapshot: ConversationSnapshotResponse | null,
    optimisticSend: OptimisticSendState | null,
): ConversationTimelineEntry[] {
    if (!snapshot) {
        if (!optimisticSend) {
            return []
        }
        return [
            {
                id: `${optimisticSend.conversationId}:optimistic:user`,
                kind: 'message',
                role: 'user',
                content: optimisticSend.message,
                timestamp: optimisticSend.createdAt,
                status: 'complete',
            },
        ]
    }

    const timeline: ConversationTimelineEntry[] = []
    const segmentsByTurn = new Map<string, ConversationSegmentResponse[]>()
    snapshot.segments.forEach((segment) => {
        const entries = segmentsByTurn.get(segment.turn_id) || []
        entries.push(segment)
        segmentsByTurn.set(segment.turn_id, entries)
    })
    snapshot.turns.forEach((turn) => {
        if (turn.role === 'user' || turn.role === 'assistant') {
            if (turn.role === 'assistant') {
                timeline.push(...buildAssistantTimelineEntries(turn, segmentsByTurn.get(turn.id) || []))
                return
            }
            timeline.push({
                id: turn.id,
                kind: 'message',
                role: turn.role,
                content: turn.content,
                timestamp: turn.timestamp,
                status: turn.status,
                error: turn.error ?? null,
            })
        }
    })

    if (!optimisticSend) {
        return timeline
    }

    return [
        ...timeline,
        {
            id: `${optimisticSend.conversationId}:optimistic:user`,
            kind: 'message',
            role: 'user',
            content: optimisticSend.message,
            timestamp: optimisticSend.createdAt,
            status: 'complete',
        },
    ]
}
