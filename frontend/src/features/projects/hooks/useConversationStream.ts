import { useEffect, useRef } from 'react'
import {
    ApiHttpError,
    fetchConversationPageValidated,
    parseConversationBatchResponse,
    parseConversationPageResponse,
    parseConversationResetResponse,
    parseConversationStreamDeltaEventResponse,
    type ConversationBatch,
    type ConversationPage,
    type ConversationStreamDeltaEventResponse,
} from '@/lib/workspaceClient'
import type { NormalizedConversationRecord } from '../model/projectsHomeState'

const LIVE_ENDPOINT = '/workspace/api/live/events'

type UseConversationStreamArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
    getConversationRecord: (conversationId: string) => NormalizedConversationRecord | undefined
    applyConversationPage: (projectPath: string, page: ConversationPage, mode: 'window' | 'older', source?: string) => void
    applyConversationBatch: (
        projectPath: string,
        conversationId: string,
        batch: ConversationBatch,
        kind: 'live' | 'mutation',
        source?: string,
    ) => void
    applyTransientConversationEvent: (
        projectPath: string,
        event: ConversationStreamDeltaEventResponse,
        source?: string,
    ) => unknown
    resetConversation: (conversationId: string) => void
    markConversationUnavailable: (conversationId: string) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    setPanelError: (message: string | null) => void
}

/**
 * Opens the active conversation at its newest page and keeps it live: a `conversation.window`
 * replaces the loaded window, `conversation.changes` merge and advance the live cursor, and a
 * `conversation.reset` stops applying until the window is reloaded.
 */
export function useConversationStream(args: UseConversationStreamArgs) {
    const { activeConversationId, activeProjectPath } = args
    const argsRef = useRef(args)

    useEffect(() => {
        argsRef.current = args
    })

    useEffect(() => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        let isCancelled = false
        let issued = 0
        // A conversation missing on a reopen after a reset is gone; on a first open it is a new thread.
        let afterReset = false

        const loadWindow = async () => {
            const sequence = ++issued
            try {
                const page = await fetchConversationPageValidated(activeConversationId, activeProjectPath)
                if (!isCancelled && sequence === issued) {
                    afterReset = false
                    argsRef.current.applyConversationPage(activeProjectPath, page, 'window', 'page-fetch')
                }
            } catch (error) {
                if (isCancelled || sequence !== issued) {
                    return
                }
                if (error instanceof ApiHttpError && error.status === 404) {
                    if (afterReset) {
                        argsRef.current.markConversationUnavailable(activeConversationId)
                    }
                    return
                }
                argsRef.current.setPanelError(argsRef.current.formatErrorMessage(error, 'Unable to load project conversation.'))
            }
        }

        const reset = () => {
            afterReset = true
            argsRef.current.resetConversation(activeConversationId)
            void loadWindow()
        }

        void loadWindow()

        const handleLiveEvent = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            // The envelope's project_path may be the server's spelling of the project; the id is the scope.
            if (isCancelled || !detail || detail.conversationId !== activeConversationId) {
                return
            }
            try {
                if (detail.type === 'conversation.window') {
                    const page = parseConversationPageResponse(detail.payload, LIVE_ENDPOINT)
                    // The server sends an empty window for a missing conversation too, so only a
                    // window holding the conversation row proves it still exists after a reset.
                    if (page.rows.some((row) => row.kind === 'conversation' && !row.deleted)) {
                        afterReset = false
                    }
                    // The stream's window is newer than any page request still in flight, which
                    // could even be another store generation's: it supersedes them. After a reset,
                    // an empty window still waits for the reload to say whether the chat is gone.
                    if (!afterReset) {
                        issued += 1
                    }
                    argsRef.current.applyConversationPage(activeProjectPath, page, 'window', 'event-stream-window')
                } else if (detail.type === 'conversation.changes') {
                    const batch = parseConversationBatchResponse(detail.payload, LIVE_ENDPOINT)
                    const record = argsRef.current.getConversationRecord(activeConversationId)
                    if (record && record.generation !== batch.generation) {
                        reset()
                        return
                    }
                    argsRef.current.applyConversationBatch(activeProjectPath, activeConversationId, batch, 'live', 'event-stream')
                } else if (detail.type === 'conversation.reset') {
                    parseConversationResetResponse(detail.payload, LIVE_ENDPOINT)
                    reset()
                } else if (detail.type === 'conversation.stream_delta') {
                    // Transient deltas render live but are droppable and never committed.
                    const delta = parseConversationStreamDeltaEventResponse(detail.payload)
                    if (delta) {
                        argsRef.current.applyTransientConversationEvent(activeProjectPath, delta, 'event-stream')
                    }
                }
            } catch {
                // A malformed frame is dropped whole; the next window or batch restores state.
            }
        }

        window.addEventListener('spark:conversation-live-event', handleLiveEvent)

        return () => {
            isCancelled = true
            window.removeEventListener('spark:conversation-live-event', handleLiveEvent)
        }
    }, [activeConversationId, activeProjectPath])
}
