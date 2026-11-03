import { useCallback, useEffect, useRef } from 'react'

import { useStore } from '@/store'
import type {
    ConversationBatch,
    ConversationPage,
    ConversationStreamDeltaEventResponse,
} from '@/lib/workspaceClient'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import {
    applyConversationBatchToCache,
    applyConversationPageToCache,
    applySummarySnapshotToCache,
    applyTransientConversationEventToCache,
    markConversationUnavailableInCache,
    resetConversationInCache,
} from '../model/projectsHomeState'
import { debugProjectChat, summarizeConversationTurnsForDebug } from '../model/projectChatDebug'

type PersistProjectState = (
    projectPath: string,
    patch: {
        last_accessed_at?: string | null
        active_conversation_id?: string | null
        is_favorite?: boolean | null
    },
) => Promise<void>

type UpdateProjectSessionState = (
    projectPath: string,
    patch: Record<string, unknown>,
) => void

type UseProjectConversationCacheArgs = {
    persistProjectState: PersistProjectState
    projectSessionsByPath: Record<string, { conversationId: string | null }>
    updateProjectSessionState: UpdateProjectSessionState
}

// Every caller commits to the store, so it, not a render's snapshot, is current.
const currentCache = () => useStore.getState().homeConversationCache

export function useProjectConversationCache({
    persistProjectState,
    projectSessionsByPath,
    updateProjectSessionState,
}: UseProjectConversationCacheArgs) {
    const conversationCache = useStore((state) => state.homeConversationCache)
    const commitHomeConversationCache = useStore((state) => state.commitHomeConversationCache)
    const setHomeThreadSummariesStatus = useStore((state) => state.setHomeThreadSummariesStatus)
    const projectSessionsRef = useRef(projectSessionsByPath)

    useEffect(() => {
        projectSessionsRef.current = projectSessionsByPath
    }, [projectSessionsByPath])

    const commitConversationCache = useCallback((
        next:
            | typeof conversationCache
            | ((current: typeof conversationCache) => typeof conversationCache),
    ) => {
        commitHomeConversationCache(typeof next === 'function' ? next(currentCache()) : next)
    }, [commitHomeConversationCache])

    // A newer request for the same project supersedes an older one, whatever order they land in.
    const summaryRequestEpochs = useRef<Record<string, number>>({})

    const loadProjectConversationSummaries = useCallback(async (projectPath: string) => {
        const epoch = (summaryRequestEpochs.current[projectPath] ?? 0) + 1
        summaryRequestEpochs.current[projectPath] = epoch
        setHomeThreadSummariesStatus(projectPath, 'loading', null)
        try {
            const snapshot = await fetchProjectConversationListValidated(projectPath)
            if (summaryRequestEpochs.current[projectPath] === epoch) {
                // Rows the live feed applied while the request was in flight outrank it by revision.
                commitConversationCache((current) => applySummarySnapshotToCache(current, snapshot, 'http', projectPath))
                setHomeThreadSummariesStatus(projectPath, 'ready', null)
            }
        } catch {
            if (summaryRequestEpochs.current[projectPath] === epoch) {
                setHomeThreadSummariesStatus(projectPath, 'error', 'Unable to load threads.')
            }
        }
        return currentCache().summariesByProjectPath[projectPath] ?? []
    }, [commitConversationCache, setHomeThreadSummariesStatus])

    // A response that answers the open chat keeps it open in its project.
    const syncActiveConversation = useCallback((projectPath: string, conversationId: string) => {
        const latestProjectScope = projectSessionsRef.current[projectPath]
        updateProjectSessionState(projectPath, { conversationId })
        if (latestProjectScope?.conversationId !== conversationId) {
            void persistProjectState(projectPath, {
                active_conversation_id: conversationId,
                last_accessed_at: new Date().toISOString(),
            })
        }
    }, [persistProjectState, updateProjectSessionState])

    /** A `window` page replaces the loaded window; an `older` page extends it. */
    const applyConversationPage = useCallback((
        projectPath: string,
        page: ConversationPage,
        mode: 'window' | 'older',
        source = 'unknown',
    ) => {
        commitConversationCache((current) => applyConversationPageToCache(current, page, mode, projectPath))
        const record = currentCache().conversationsById[page.conversation_id]
        debugProjectChat('apply conversation page', {
            source,
            projectPath,
            mode,
            conversationId: page.conversation_id,
            watermark: page.watermark,
            turns: record ? summarizeConversationTurnsForDebug(record.orderedTurnIds.map((turnId) => record.turnsById[turnId])) : [],
        })
    }, [commitConversationCache])

    /**
     * Merges a batch's rows by revision. `live` batches come from the conversation feed and move
     * its cursor; `mutation` batches are a request's own results and never do.
     */
    const applyConversationBatch = useCallback((
        projectPath: string,
        conversationId: string,
        batch: ConversationBatch,
        kind: 'live' | 'mutation',
        source = 'unknown',
        options?: { forceWorkspaceSync?: boolean },
    ) => {
        commitConversationCache((current) => applyConversationBatchToCache(current, conversationId, batch, kind, projectPath))
        debugProjectChat('apply conversation batch', {
            source,
            projectPath,
            kind,
            conversationId,
            fromRevision: batch.from_revision,
            watermark: batch.watermark,
            rowCount: batch.rows.length,
        })
        if (options?.forceWorkspaceSync && currentCache().conversationsById[conversationId]) {
            syncActiveConversation(projectPath, conversationId)
        }
    }, [commitConversationCache, syncActiveConversation])

    const resetConversation = useCallback((conversationId: string) => {
        commitConversationCache((current) => resetConversationInCache(current, conversationId))
    }, [commitConversationCache])

    const markConversationUnavailable = useCallback((conversationId: string) => {
        commitConversationCache((current) => markConversationUnavailableInCache(current, conversationId))
    }, [commitConversationCache])

    const applyTransientConversationEvent = useCallback((
        projectPath: string,
        event: ConversationStreamDeltaEventResponse,
        source = 'unknown',
    ) => {
        const result = applyTransientConversationEventToCache(currentCache(), event)
        if (result.status === 'dropped') {
            return result
        }
        commitConversationCache(result.cache)
        debugProjectChat('apply transient conversation stream delta', {
            source,
            projectPath,
            conversationId: event.conversation_id,
            deltaKind: event.delta_kind,
            streamSequence: event.stream_sequence,
        })
        return result
    }, [commitConversationCache])

    return {
        applyConversationBatch,
        applyConversationPage,
        applyTransientConversationEvent,
        commitConversationCache,
        conversationCache,
        loadProjectConversationSummaries,
        markConversationUnavailable,
        resetConversation,
    }
}
