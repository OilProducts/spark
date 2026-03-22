import type {
    ConversationSegmentResponse,
    ConversationSegmentUpsertEventResponse,
    ConversationSnapshotResponse,
    ConversationSummaryResponse,
    ConversationTurnResponse,
    ConversationTurnUpsertEventResponse,
    ExecutionCardResponse,
    SpecEditProposalResponse,
} from '@/lib/workspaceClient'

export type OptimisticSendState = {
    conversationId: string
    message: string
    createdAt: string
}

export type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

export function getLatestApprovedSpecEditProposal(snapshot: ConversationSnapshotResponse | null): SpecEditProposalResponse | null {
    if (!snapshot) {
        return null
    }
    for (let index = snapshot.spec_edit_proposals.length - 1; index >= 0; index -= 1) {
        const proposal = snapshot.spec_edit_proposals[index]
        if (proposal?.status === 'applied') {
            return proposal
        }
    }
    return null
}

export function getLatestExecutionCard(snapshot: ConversationSnapshotResponse | null): ExecutionCardResponse | null {
    if (!snapshot || snapshot.execution_cards.length === 0) {
        return null
    }
    return snapshot.execution_cards[snapshot.execution_cards.length - 1] || null
}

export function ensureConversationSnapshotShell(
    conversationId: string,
    projectPath: string,
    title = 'New thread',
): ConversationSnapshotResponse {
    return {
        schema_version: 4,
        conversation_id: conversationId,
        conversation_handle: '',
        project_path: projectPath,
        title,
        created_at: '',
        updated_at: '',
        turns: [],
        segments: [],
        event_log: [],
        spec_edit_proposals: [],
        flow_run_requests: [],
        flow_launches: [],
        execution_cards: [],
        execution_workflow: {
            status: 'idle',
            run_id: null,
            error: null,
            flow_source: null,
        },
    }
}

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

function scoreConversationSnapshotFreshness(snapshot: ConversationSnapshotResponse): number {
    const turnStatusScore = snapshot.turns.reduce((score, turn) => {
        if (turn.status === 'failed') {
            return score + 4
        }
        if (turn.status === 'complete') {
            return score + 3
        }
        if (turn.status === 'streaming') {
            return score + 2
        }
        return score + 1
    }, 0)
    const contentScore = snapshot.turns.reduce((score, turn) => score + turn.content.length, 0)
    return (
        snapshot.turns.length * 100000
        + snapshot.segments.length * 1000
        + turnStatusScore * 100
        + contentScore
    )
}

export function compareConversationSnapshotFreshness(
    left: ConversationSnapshotResponse,
    right: ConversationSnapshotResponse,
): number {
    const updatedAtCompare = left.updated_at.localeCompare(right.updated_at)
    if (updatedAtCompare !== 0) {
        return updatedAtCompare
    }
    return scoreConversationSnapshotFreshness(left) - scoreConversationSnapshotFreshness(right)
}

export function sortConversationSummaries(items: ConversationSummaryResponse[]): ConversationSummaryResponse[] {
    return [...items].sort((left, right) => right.updated_at.localeCompare(left.updated_at))
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
