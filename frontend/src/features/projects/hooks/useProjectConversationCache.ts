import { useCallback, useEffect, useRef } from 'react'

import { useStore } from '@/store'
import type {
    ConversationSummaryResponse,
    ConversationSegmentUpsertEventResponse,
    ConversationSnapshotResponse,
    ConversationTurnUpsertEventResponse,
} from '@/lib/workspaceClient'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import {
    applyConversationSnapshotToCache,
    applyConversationStreamEventToCache,
    setProjectConversationSummaryList,
    type ConversationStreamEvent,
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

export function useProjectConversationCache({
    persistProjectState,
    projectSessionsByPath,
    updateProjectSessionState,
}: UseProjectConversationCacheArgs) {
    const conversationCache = useStore((state) => state.homeConversationCache)
    const commitHomeConversationCache = useStore((state) => state.commitHomeConversationCache)
    const setHomeThreadSummariesStatus = useStore((state) => state.setHomeThreadSummariesStatus)
    const conversationCacheRef = useRef(conversationCache)
    const projectSessionsRef = useRef(projectSessionsByPath)

    useEffect(() => {
        conversationCacheRef.current = conversationCache
    }, [conversationCache])

    useEffect(() => {
        projectSessionsRef.current = projectSessionsByPath
    }, [projectSessionsByPath])

    const commitConversationCache = useCallback((
        next:
            | typeof conversationCache
            | ((current: typeof conversationCache) => typeof conversationCache),
    ) => {
        const nextCache = typeof next === 'function'
            ? next(conversationCacheRef.current)
            : next
        conversationCacheRef.current = nextCache
        commitHomeConversationCache(nextCache)
    }, [commitHomeConversationCache])

    const setConversationSummaryList = useCallback((projectPath: string, summaries: ConversationSummaryResponse[]) => {
        const nextCache = setProjectConversationSummaryList(conversationCacheRef.current, projectPath, summaries)
        commitConversationCache(nextCache)
    }, [commitConversationCache])

    const loadProjectConversationSummaries = useCallback(async (projectPath: string) => {
        setHomeThreadSummariesStatus(projectPath, 'loading', null)
        try {
            const summaries = await fetchProjectConversationListValidated(projectPath)
            setConversationSummaryList(projectPath, summaries)
            setHomeThreadSummariesStatus(projectPath, 'ready', null)
            return summaries
        } catch {
            setHomeThreadSummariesStatus(projectPath, 'error', 'Unable to load threads.')
            return conversationCacheRef.current.summariesByProjectPath[projectPath] || []
        }
    }, [setConversationSummaryList, setHomeThreadSummariesStatus])

    const applyConversationSnapshot = useCallback((
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source = 'unknown',
        options?: {
            forceWorkspaceSync?: boolean
        },
    ) => {
        const latestProjectScope = projectSessionsRef.current[projectPath]
        const shouldSyncActiveWorkspace = options?.forceWorkspaceSync === true
            || latestProjectScope?.conversationId === snapshot.conversation_id
        const result = applyConversationSnapshotToCache(
            conversationCacheRef.current,
            projectPath,
            snapshot,
        )
        if (!result.applied) {
            debugProjectChat('skip stale conversation snapshot', {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                snapshotUpdatedAt: snapshot.updated_at,
            })
            return
        }
        const { cache } = result
        const record = cache.conversationsById[snapshot.conversation_id]
        if (!record) {
            return
        }
        debugProjectChat('apply conversation snapshot', {
            source,
            projectPath,
            snapshotProjectPath: snapshot.project_path,
            conversationId: snapshot.conversation_id,
            shouldSyncActiveWorkspace,
            turnCount: record.orderedTurnIds.length,
            turns: summarizeConversationTurnsForDebug(record.orderedTurnIds.map((turnId) => record.turnsById[turnId])),
        })
        commitConversationCache(cache)

        if (shouldSyncActiveWorkspace) {
            updateProjectSessionState(projectPath, {
                conversationId: snapshot.conversation_id,
                projectEventLog: snapshot.event_log.map((entry) => ({
                    message: entry.message,
                    timestamp: entry.timestamp,
                })),
            })
            if (latestProjectScope?.conversationId !== snapshot.conversation_id) {
                void persistProjectState(projectPath, {
                    active_conversation_id: snapshot.conversation_id,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        }
    }, [commitConversationCache, persistProjectState, updateProjectSessionState])

    const applyConversationStreamEvent = useCallback((
        projectPath: string,
        event: ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse,
        source = 'unknown',
    ) => {
        debugProjectChat('apply conversation stream event', {
            source,
            projectPath,
            eventType: event.type,
            conversationId: event.conversation_id,
        })
        const { cache, record } = applyConversationStreamEventToCache(
            conversationCacheRef.current,
            projectPath,
            event as ConversationStreamEvent,
        )
        commitConversationCache(cache)
        debugProjectChat('apply merged stream conversation record', {
            source,
            projectPath,
            conversationId: record.conversation_id,
            turnCount: record.orderedTurnIds.length,
        })
    }, [commitConversationCache])

    return {
        applyConversationSnapshot,
        applyConversationStreamEvent,
        commitConversationCache,
        conversationCache,
        conversationCacheRef,
        loadProjectConversationSummaries,
        setConversationSummaryList,
    }
}
