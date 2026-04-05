import { useCallback, useEffect, useRef, type Dispatch, type SetStateAction } from 'react'

import { useStore } from '@/store'
import type {
    ConversationSegmentUpsertEventResponse,
    ConversationSnapshotResponse,
    ConversationTurnUpsertEventResponse,
} from '@/lib/workspaceClient'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import type { ProjectGitMetadata } from '../model/presentation'
import {
    applyConversationSnapshotToCache,
    applyConversationStreamEventToCache,
    derivePlanStatusFromExecutionCard,
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
    setProjectGitMetadata: Dispatch<SetStateAction<Record<string, ProjectGitMetadata>>>
    updateProjectSessionState: UpdateProjectSessionState
}

export function useProjectConversationCache({
    persistProjectState,
    projectSessionsByPath,
    setProjectGitMetadata,
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

    const setConversationSummaryList = useCallback((projectPath: string, summaries: ProjectGitMetadata extends never ? never : import('@/lib/workspaceClient').ConversationSummaryResponse[]) => {
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
        const { applied, cache, latestApprovedProposal, latestExecutionCard } = applyConversationSnapshotToCache(
            conversationCacheRef.current,
            projectPath,
            snapshot,
        )
        if (!applied) {
            debugProjectChat('skip stale conversation snapshot', {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                snapshotUpdatedAt: snapshot.updated_at,
            })
            return
        }
        debugProjectChat('apply conversation snapshot', {
            source,
            projectPath,
            snapshotProjectPath: snapshot.project_path,
            conversationId: snapshot.conversation_id,
            shouldSyncActiveWorkspace,
            turnCount: snapshot.turns.length,
            turns: summarizeConversationTurnsForDebug(snapshot.turns),
        })
        commitConversationCache(cache)

        if (shouldSyncActiveWorkspace) {
            updateProjectSessionState(projectPath, {
                conversationId: snapshot.conversation_id,
                projectEventLog: snapshot.event_log.map((entry) => ({
                    message: entry.message,
                    timestamp: entry.timestamp,
                })),
                specId: latestApprovedProposal?.canonical_spec_edit_id ?? null,
                specStatus: latestApprovedProposal ? 'approved' : 'draft',
                specProvenance: latestApprovedProposal
                    ? {
                        source: 'spec-edit-proposal',
                        referenceId: latestApprovedProposal.id,
                        capturedAt: latestApprovedProposal.approved_at || latestApprovedProposal.created_at,
                        runId: null,
                        gitBranch: latestApprovedProposal.git_branch ?? null,
                        gitCommit: latestApprovedProposal.git_commit ?? null,
                    }
                    : null,
                planId: latestExecutionCard?.id ?? null,
                planStatus: derivePlanStatusFromExecutionCard(latestExecutionCard),
                planProvenance: latestExecutionCard
                    ? {
                        source: 'execution-card',
                        referenceId: latestExecutionCard.id,
                        capturedAt: latestExecutionCard.updated_at,
                        runId: latestExecutionCard.source_workflow_run_id,
                        gitBranch: latestApprovedProposal?.git_branch ?? null,
                        gitCommit: latestApprovedProposal?.git_commit ?? null,
                    }
                    : null,
            })
            if (latestProjectScope?.conversationId !== snapshot.conversation_id) {
                void persistProjectState(projectPath, {
                    active_conversation_id: snapshot.conversation_id,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        }

        if (latestApprovedProposal?.git_branch || latestApprovedProposal?.git_commit) {
            setProjectGitMetadata((current) => ({
                ...current,
                [projectPath]: {
                    branch: latestApprovedProposal.git_branch ?? current[projectPath]?.branch ?? null,
                    commit: latestApprovedProposal.git_commit ?? current[projectPath]?.commit ?? null,
                },
            }))
        }
    }, [commitConversationCache, persistProjectState, setProjectGitMetadata, updateProjectSessionState])

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
        const { cache, snapshot } = applyConversationStreamEventToCache(
            conversationCacheRef.current,
            projectPath,
            event as ConversationStreamEvent,
        )
        commitConversationCache(cache)
        if (snapshot) {
            debugProjectChat('apply merged stream snapshot', {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                turnCount: snapshot.turns.length,
            })
        }
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
