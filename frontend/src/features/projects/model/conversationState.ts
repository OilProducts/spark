import type {
    ConversationSegmentResponse,
    ConversationSegmentUpsertEventResponse,
    ConversationSnapshotResponse,
    ConversationSummaryResponse,
    ConversationTurnResponse,
    ConversationTurnUpsertEventResponse,
} from '@/lib/workspaceClient'

export type OptimisticSendState = {
    conversationId: string
    message: string
    createdAt: string
}

export type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

export function upsertConversationTurn(
    snapshot: ConversationSnapshotResponse,
    turn: ConversationTurnResponse,
): ConversationSnapshotResponse {
    const nextTurns = [...snapshot.turns]
    const existingIndex = nextTurns.findIndex((entry) => entry.id === turn.id)
    if (existingIndex >= 0) {
        nextTurns[existingIndex] = turn
    } else {
        nextTurns.push(turn)
    }
    return {
        ...snapshot,
        turns: nextTurns,
    }
}

export function upsertConversationSegment(
    snapshot: ConversationSnapshotResponse,
    segment: ConversationSegmentResponse,
): ConversationSnapshotResponse {
    const nextSegments = [...snapshot.segments]
    const existingIndex = nextSegments.findIndex((entry) => entry.id === segment.id)
    if (existingIndex >= 0) {
        nextSegments[existingIndex] = segment
    } else {
        nextSegments.push(segment)
    }
    nextSegments.sort((left, right) => {
        if (left.turn_id === right.turn_id) {
            const orderDelta = left.order - right.order
            if (orderDelta !== 0) {
                return orderDelta
            }
            const timestampDelta = left.timestamp.localeCompare(right.timestamp)
            if (timestampDelta !== 0) {
                return timestampDelta
            }
            return left.id.localeCompare(right.id)
        }
        return left.timestamp.localeCompare(right.timestamp)
    })
    return {
        ...snapshot,
        segments: nextSegments,
    }
}

export function sanitizeStreamingTurnUpsert(
    currentTurn: ConversationTurnResponse | null,
    incomingTurn: ConversationTurnResponse,
): ConversationTurnResponse {
    if (incomingTurn.role !== 'assistant') {
        return incomingTurn
    }
    if (incomingTurn.status !== 'pending' && incomingTurn.status !== 'streaming') {
        return incomingTurn
    }
    if (incomingTurn.content.trim().length > 0) {
        return incomingTurn
    }
    return {
        ...incomingTurn,
        content: currentTurn?.content ?? '',
    }
}

export function sortConversationSummaries(items: ConversationSummaryResponse[]): ConversationSummaryResponse[] {
    return [...items].sort((left, right) => {
        const updatedAtCompare = right.updated_at.localeCompare(left.updated_at)
        return updatedAtCompare !== 0 ? updatedAtCompare : right.revision - left.revision
    })
}

export function upsertConversationSummary(
    items: ConversationSummaryResponse[],
    summary: ConversationSummaryResponse,
): ConversationSummaryResponse[] {
    return sortConversationSummaries([
        summary,
        ...items.filter((entry) => entry.conversation_id !== summary.conversation_id),
    ])
}
