import { useEffect, useRef } from 'react'
import {
    ApiHttpError,
    fetchConversationSnapshotValidated,
    parseConversationSnapshotResponse,
    parseConversationStreamEventResponse,
    type ConversationSnapshotResponse,
    type ConversationTurnUpsertEventResponse,
    type ConversationSegmentUpsertEventResponse,
} from '@/lib/workspaceClient'
import type { ApplyConversationStreamEventResult } from '../model/projectsHomeState'

type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

type UseConversationStreamArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
    applyConversationSnapshot: (projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => unknown
    applyConversationStreamEvent: (
        projectPath: string,
        event: ConversationStreamEvent,
        source?: string,
    ) => ApplyConversationStreamEventResult | undefined
    formatErrorMessage: (error: unknown, fallback: string) => string
    setPanelError: (message: string | null) => void
}

export function useConversationStream({
    activeConversationId,
    activeProjectPath,
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
        const hasMatchingSidecar = segmentKind === 'plan'
            ? event.proposed_plans?.some((plan) => plan.id === event.segment.artifact_id)
            : segmentKind === 'flow_run_request'
                ? event.flow_run_requests?.some((request) => request.id === event.segment.artifact_id)
                : event.flow_launches?.some((launch) => launch.id === event.segment.artifact_id)
        if (hasMatchingSidecar) {
            return
        }
        void refreshSnapshot()
    }

    const snapshotHandlerRef = useRef(applyConversationSnapshot)
    const eventHandlerRef = useRef(applyConversationStreamEvent)
    const errorFormatterRef = useRef(formatErrorMessage)
    const setPanelErrorRef = useRef(setPanelError)

    useEffect(() => {
        snapshotHandlerRef.current = applyConversationSnapshot
        eventHandlerRef.current = applyConversationStreamEvent
        errorFormatterRef.current = formatErrorMessage
        setPanelErrorRef.current = setPanelError
    }, [
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
        let snapshotFetchInFlight: Promise<void> | null = null
        let snapshotFetchMarker: object | null = null
        let pendingPreSnapshotEvents: ConversationStreamEvent[] = []

        const replayPendingEventsAfterSnapshot = (snapshot: ConversationSnapshotResponse) => {
            const replayableEvents = pendingPreSnapshotEvents
                .filter((pendingEvent) => pendingEvent.revision > snapshot.revision)
                .sort((left, right) => left.revision - right.revision)
            pendingPreSnapshotEvents = []
            replayableEvents.forEach((pendingEvent) => {
                eventHandlerRef.current(activeProjectPath, pendingEvent, 'event-stream-replay')
                maybeRefreshSnapshotForArtifactSegment(pendingEvent, loadSnapshot)
            })
        }

        const loadSnapshot = async () => {
            if (snapshotFetchInFlight) {
                return snapshotFetchInFlight
            }
            const currentFetchMarker = {}
            snapshotFetchMarker = currentFetchMarker
            const currentFetch = (async () => {
                try {
                    const snapshot = await fetchConversationSnapshotValidated(activeConversationId, activeProjectPath)
                    if (isCancelled) {
                        return
                    }
                    snapshotHandlerRef.current(activeProjectPath, snapshot, 'snapshot-fetch')
                    snapshotFetchInFlight = null
                    snapshotFetchMarker = null
                    replayPendingEventsAfterSnapshot(snapshot)
                } catch (error) {
                    if (isCancelled) {
                        return
                    }
                    if (error instanceof ApiHttpError && error.status === 404) {
                        return
                    }
                    const message = errorFormatterRef.current(error, 'Unable to load project conversation.')
                    setPanelErrorRef.current(message)
                } finally {
                    if (snapshotFetchMarker === currentFetchMarker) {
                        snapshotFetchInFlight = null
                        snapshotFetchMarker = null
                    }
                }
            })()
            snapshotFetchInFlight = currentFetch
            return snapshotFetchInFlight
        }

        void loadSnapshot()

        const handleLiveEvent = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            if (
                !detail
                || detail.conversationId !== activeConversationId
                || detail.projectPath !== activeProjectPath
            ) {
                return
            }
            if (isCancelled) {
                return
            }
            try {
                const payload = detail.payload as { type?: string; state?: unknown }
                if (detail.type === 'resync_required') {
                    void loadSnapshot()
                    return
                }
                if (payload.type === 'conversation_snapshot') {
                    const snapshot = parseConversationSnapshotResponse(
                        payload.state,
                        '/workspace/api/live/events',
                    )
                    snapshotHandlerRef.current(activeProjectPath, snapshot, 'event-stream-snapshot')
                    replayPendingEventsAfterSnapshot(snapshot)
                    return
                }
                if (detail.type === 'conversation.snapshot' && payload.state) {
                    const snapshot = parseConversationSnapshotResponse(
                        payload.state,
                        '/workspace/api/live/events',
                    )
                    snapshotHandlerRef.current(activeProjectPath, snapshot, 'event-stream-snapshot')
                    replayPendingEventsAfterSnapshot(snapshot)
                    return
                }
                const parsedEvent = parseConversationStreamEventResponse(
                    payload,
                    '/workspace/api/live/events',
                )
                if (!parsedEvent) {
                    return
                }
                const result = eventHandlerRef.current(activeProjectPath, parsedEvent, 'event-stream')
                if (result?.status === 'missing_record') {
                    pendingPreSnapshotEvents.push(parsedEvent)
                    void loadSnapshot()
                    return
                }
                maybeRefreshSnapshotForArtifactSegment(parsedEvent, loadSnapshot)
            } catch {
                // Ignore malformed stream events.
            }
        }

        window.addEventListener('spark:conversation-live-event', handleLiveEvent)

        return () => {
            isCancelled = true
            window.removeEventListener('spark:conversation-live-event', handleLiveEvent)
        }
    }, [activeConversationId, activeProjectPath])
}
