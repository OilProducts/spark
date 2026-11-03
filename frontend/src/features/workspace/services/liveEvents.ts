import { workspaceLiveEventsUrl } from '@/lib/api/apiClient'
import {
    parseConversationBatchResponse,
    parseConversationResetResponse,
    parseConversationSummarySnapshotResponse,
    type ConversationBatch,
    type ConversationReset,
    type ConversationSummarySnapshot,
} from '@/lib/api/conversationsApi'

const LIVE_ENDPOINT = '/workspace/api/live/events'

export function buildWorkspaceLiveEventsUrl(params: URLSearchParams): string {
    return workspaceLiveEventsUrl(params)
}

export type LiveEnvelope = {
    type?: string
    project_path?: string | null
    resource?: { kind?: string; id?: string | null }
    payload?: unknown
}

export type ConversationLiveHandlers = {
    /** `conversations.snapshot`: replace the feed's summaries and set its cursor. */
    onSummarySnapshot: (snapshot: ConversationSummarySnapshot) => void
    /** `conversations.changes`: merge summary rows and advance the cursor, even when empty. */
    onSummaryChanges: (batch: ConversationBatch) => void
    /** `conversations.reset`: stop applying, clear the cursor and reconnect without it. */
    onSummaryReset: (reset: ConversationReset) => void
    /** `conversation.reset`: forget that conversation's cursor and reconnect without it. */
    onConversationReset: (conversationId: string) => void
}

/**
 * Routes the conversation envelopes. Summary-feed envelopes are parsed for their handlers;
 * a single conversation's `conversation.window|changes|reset` and stream deltas go to its
 * stream as a `spark:conversation-live-event`. Returns whether the envelope was one of them;
 * a malformed payload throws.
 */
export function dispatchConversationLiveEnvelope(envelope: LiveEnvelope, handlers: ConversationLiveHandlers): boolean {
    switch (envelope.type) {
    case 'conversations.snapshot':
        handlers.onSummarySnapshot(parseConversationSummarySnapshotResponse(envelope.payload, LIVE_ENDPOINT))
        return true
    case 'conversations.changes':
        handlers.onSummaryChanges(parseConversationBatchResponse(envelope.payload, LIVE_ENDPOINT))
        return true
    case 'conversations.reset':
        handlers.onSummaryReset(parseConversationResetResponse(envelope.payload, LIVE_ENDPOINT))
        return true
    }
    if (envelope.resource?.kind !== 'conversation' || !envelope.resource.id) {
        return false
    }
    const conversationId = envelope.resource.id
    window.dispatchEvent(new CustomEvent('spark:conversation-live-event', {
        detail: {
            type: envelope.type,
            conversationId,
            projectPath: envelope.project_path ?? null,
            payload: envelope.payload,
        },
    }))
    if (envelope.type === 'conversation.reset') {
        handlers.onConversationReset(conversationId)
    }
    return true
}
