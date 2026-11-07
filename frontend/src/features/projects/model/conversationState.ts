import {
    compareRevisions,
    type ConversationRow,
    type ConversationSummaryResponse,
} from '@/lib/workspaceClient'
import type { ConversationTimelineEntry } from './types'

export type PendingConversationTurnState = {
    conversationId: string
    /** The conversation's newest applied row revision when the turn was sent. */
    afterRevision: string
}

/** A sent message, shown until the server records it under the same id. */
export type PendingUserMessage = Extract<ConversationTimelineEntry, { kind: 'message' }>

/** Rows by identity: `(kind, conversation_id, collection when artifact, id)`. */
export type ConversationRows = Record<string, ConversationRow>

export function conversationRowKey(row: Pick<ConversationRow, 'kind' | 'conversation_id' | 'id'> & { collection?: string }): string {
    return row.kind === 'artifact'
        ? `artifact\u0000${row.conversation_id}\u0000${row.collection}\u0000${row.id}`
        : `${row.kind}\u0000${row.conversation_id}\u0000${row.id}`
}

/**
 * Applies each row newer than the one held under its identity. Equal and older revisions are
 * ignored, so replays are idempotent and a tombstone keeps an older copy from coming back.
 * Returns `rows` itself when nothing changed.
 */
export function mergeConversationRows(rows: ConversationRows, incoming: ConversationRow[]): ConversationRows {
    let next = rows
    for (const row of incoming) {
        const key = conversationRowKey(row)
        const current = next[key]
        if (current && compareRevisions(row.revision, current.revision) <= 0) {
            continue
        }
        if (next === rows) {
            next = { ...rows }
        }
        next[key] = row
    }
    return next
}

/** Standing roles first, the Coordinator leading; then newest first. */
export function pinRank(item: { standing_role?: { name: string } | null }): string {
    const role = item.standing_role
    return role ? `0${role.name === 'coordinator' ? '0' : '1'}${role.name}` : '1'
}

export function sortConversationSummaries(items: ConversationSummaryResponse[]): ConversationSummaryResponse[] {
    return [...items].sort((left, right) => {
        const pinCompare = pinRank(left).localeCompare(pinRank(right))
        if (pinCompare !== 0) return pinCompare
        const updatedAtCompare = right.updated_at.localeCompare(left.updated_at)
        return updatedAtCompare !== 0 ? updatedAtCompare : compareRevisions(right.revision, left.revision)
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
