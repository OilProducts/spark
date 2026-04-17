import { useEffect, useRef } from 'react'
import {
    ApiHttpError,
    conversationEventsUrl,
    fetchConversationSnapshotValidated,
    parseConversationSnapshotResponse,
    parseConversationStreamEventResponse,
    type ConversationSnapshotResponse,
    type ConversationTurnUpsertEventResponse,
    type ConversationSegmentUpsertEventResponse,
} from '@/lib/workspaceClient'

type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

type UseConversationStreamArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
    appendLocalProjectEvent: (message: string) => void
    applyConversationSnapshot: (projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => void
    applyConversationStreamEvent: (projectPath: string, event: ConversationStreamEvent, source?: string) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    setPanelError: (message: string | null) => void
}

export function useConversationStream({
    activeConversationId,
    activeProjectPath,
    appendLocalProjectEvent,
    applyConversationSnapshot,
    applyConversationStreamEvent,
    formatErrorMessage,
    setPanelError,
}: UseConversationStreamArgs) {
    const maybeRefreshSnapshotForArtifactSegment = (
        event: ConversationStreamEvent,
        refreshSnapshot: () => Promise<void>,
    ) => {
        if (event.type !== 'segment_upsert') {
            return
        }
        const segmentKind = event.segment.kind
        if (!event.segment.artifact_id) {
            return
        }
        if (segmentKind !== 'plan' && segmentKind !== 'flow_run_request' && segmentKind !== 'flow_launch') {
            return
        }
        void refreshSnapshot()
    }

    const snapshotHandlerRef = useRef(applyConversationSnapshot)
    const eventHandlerRef = useRef(applyConversationStreamEvent)
    const errorFormatterRef = useRef(formatErrorMessage)
    const appendEventRef = useRef(appendLocalProjectEvent)
    const setPanelErrorRef = useRef(setPanelError)

    useEffect(() => {
        snapshotHandlerRef.current = applyConversationSnapshot
        eventHandlerRef.current = applyConversationStreamEvent
        errorFormatterRef.current = formatErrorMessage
        appendEventRef.current = appendLocalProjectEvent
        setPanelErrorRef.current = setPanelError
    }, [
        appendLocalProjectEvent,
        applyConversationSnapshot,
        applyConversationStreamEvent,
        formatErrorMessage,
        setPanelError,
    ])

    useEffect(() => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        let isCancelled = false
        let eventSource: EventSource | null = null

        const loadSnapshot = async () => {
            try {
                const snapshot = await fetchConversationSnapshotValidated(activeConversationId, activeProjectPath)
                if (isCancelled) {
                    return
                }
                snapshotHandlerRef.current(activeProjectPath, snapshot, 'snapshot-fetch')
            } catch (error) {
                if (isCancelled) {
                    return
                }
                if (error instanceof ApiHttpError && error.status === 404) {
                    return
                }
                const message = errorFormatterRef.current(error, 'Unable to load project conversation.')
                setPanelErrorRef.current(message)
                appendEventRef.current(`Project chat sync failed: ${message}`)
            }
        }

        void loadSnapshot()

        if (typeof EventSource !== 'undefined') {
            const eventStreamUrl = conversationEventsUrl(activeConversationId, activeProjectPath)
            eventSource = new EventSource(eventStreamUrl)
            eventSource.onmessage = (event) => {
                if (isCancelled) {
                    return
                }
                try {
                    const payload = JSON.parse(event.data) as { type?: string; state?: unknown }
                    if (payload.type === 'conversation_snapshot') {
                        const snapshot = parseConversationSnapshotResponse(
                            payload.state,
                            '/workspace/api/conversations/{id}/events',
                        )
                        snapshotHandlerRef.current(activeProjectPath, snapshot, 'event-stream-snapshot')
                        return
                    }
                    const parsedEvent = parseConversationStreamEventResponse(
                        payload,
                        '/workspace/api/conversations/{id}/events',
                    )
                    if (!parsedEvent) {
                        return
                    }
                    eventHandlerRef.current(activeProjectPath, parsedEvent, 'event-stream')
                    maybeRefreshSnapshotForArtifactSegment(parsedEvent, loadSnapshot)
                } catch {
                    // Ignore malformed stream events.
                }
            }
        }

        return () => {
            isCancelled = true
            eventSource?.close()
        }
    }, [activeConversationId, activeProjectPath])
}
