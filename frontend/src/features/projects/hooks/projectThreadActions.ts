import { useCallback, type MutableRefObject } from 'react'

import {
    deleteConversationValidated,
    fetchProjectConversationListValidated,
    type ConversationSnapshotResponse,
    type ConversationSummaryResponse,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/ui'

import {
    buildProjectConversationId,
    extractApiErrorMessage,
    removeConversationFromCache,
    type ProjectConversationCacheState,
} from '../model/projectsHomeState'

type PersistProjectState = (
    projectPath: string,
    patch: {
        last_accessed_at?: string | null
        active_conversation_id?: string | null
        is_favorite?: boolean | null
    },
) => Promise<void>

type ConversationCacheRef = MutableRefObject<ProjectConversationCacheState>

type UseProjectThreadActionsArgs = {
    activeProjectPath: string | null
    activeConversationId: string | null
    conversationCacheRef: ConversationCacheRef
    setConversationSummaryList: (projectPath: string, summaries: ConversationSummaryResponse[]) => void
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source?: string,
        options?: { forceWorkspaceSync?: boolean },
    ) => void
    activateConversationThread: (projectPath: string, conversationId: string, source?: string) => void
    resetComposer: () => void
    setConversationId: (conversationId: string | null) => void
    updateProjectSessionState: (projectPath: string, patch: Record<string, unknown>) => void
    clearHomeConversationSession: (conversationId: string) => void
    setPanelError: (value: string | null) => void
    setPendingDeleteConversationId: (value: string | null) => void
    appendLocalProjectEvent: (message: string) => void
    commitConversationCache: (
        next:
            | ProjectConversationCacheState
            | ((current: ProjectConversationCacheState) => ProjectConversationCacheState),
    ) => void
    persistProjectState: PersistProjectState
}

export function useProjectThreadActions({
    activeProjectPath,
    activeConversationId,
    conversationCacheRef,
    setConversationSummaryList,
    applyConversationSnapshot,
    activateConversationThread,
    resetComposer,
    setConversationId,
    updateProjectSessionState,
    clearHomeConversationSession,
    setPanelError,
    setPendingDeleteConversationId,
    appendLocalProjectEvent,
    commitConversationCache,
    persistProjectState,
}: UseProjectThreadActionsArgs) {
    const { confirm } = useDialogController()

    const onCreateConversationThread = useCallback(() => {
        if (!activeProjectPath) {
            return
        }
        const now = new Date().toISOString()
        const conversationId = buildProjectConversationId(activeProjectPath)
        setPanelError(null)
        setConversationSummaryList(activeProjectPath, [
            {
                conversation_id: conversationId,
                conversation_handle: '',
                project_path: activeProjectPath,
                title: 'New thread',
                created_at: now,
                updated_at: now,
                last_message_preview: null,
            },
            ...(conversationCacheRef.current.summariesByProjectPath[activeProjectPath] || []),
        ])
        activateConversationThread(activeProjectPath, conversationId, 'create-thread')
    }, [
        activeProjectPath,
        activateConversationThread,
        conversationCacheRef,
        setConversationSummaryList,
        setPanelError,
    ])

    const onSelectConversationThread = useCallback((conversationId: string) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        activateConversationThread(activeProjectPath, conversationId, 'select-thread')
        const cachedSnapshot = conversationCacheRef.current.snapshotsByConversationId[conversationId]
        if (cachedSnapshot) {
            applyConversationSnapshot(activeProjectPath, cachedSnapshot, 'thread-cache')
        }
    }, [
        activeProjectPath,
        activateConversationThread,
        applyConversationSnapshot,
        conversationCacheRef,
        setPanelError,
    ])

    const onDeleteConversationThread = useCallback(async (conversationId: string, title: string) => {
        if (!activeProjectPath) {
            return
        }
        const confirmed = await confirm({
            title: 'Delete thread?',
            description: `Delete thread "${title}"?`,
            confirmLabel: 'Delete thread',
            cancelLabel: 'Keep thread',
            confirmVariant: 'destructive',
        })
        if (!confirmed) {
            return
        }
        setPanelError(null)
        setPendingDeleteConversationId(conversationId)
        try {
            await deleteConversationValidated(conversationId, activeProjectPath)
            commitConversationCache((current) => removeConversationFromCache(current, conversationId))
            clearHomeConversationSession(conversationId)
            const localRemainingSummaries = (
                conversationCacheRef.current.summariesByProjectPath[activeProjectPath] || []
            ).filter((entry) => entry.conversation_id !== conversationId)
            setConversationSummaryList(activeProjectPath, localRemainingSummaries)

            let remainingSummaries = localRemainingSummaries
            try {
                remainingSummaries = await fetchProjectConversationListValidated(activeProjectPath)
                setConversationSummaryList(activeProjectPath, remainingSummaries)
            } catch {
                // Keep the local optimistic removal if the follow-up refresh fails.
            }

            if (activeConversationId === conversationId) {
                const fallbackConversationId = remainingSummaries[0]?.conversation_id || null
                resetComposer()
                setConversationId(fallbackConversationId)
                if (fallbackConversationId) {
                    updateProjectSessionState(activeProjectPath, {
                        conversationId: fallbackConversationId,
                    })
                }
                void persistProjectState(activeProjectPath, {
                    active_conversation_id: fallbackConversationId,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        } catch (error) {
            const message = extractApiErrorMessage(error, 'Unable to delete the thread.')
            setPanelError(message)
            appendLocalProjectEvent(`Thread deletion failed: ${message}`)
        } finally {
            setPendingDeleteConversationId(null)
        }
    }, [
        activeConversationId,
        activeProjectPath,
        appendLocalProjectEvent,
        commitConversationCache,
        conversationCacheRef,
        persistProjectState,
        resetComposer,
        setConversationId,
        setConversationSummaryList,
        clearHomeConversationSession,
        setPanelError,
        setPendingDeleteConversationId,
        updateProjectSessionState,
        confirm,
    ])

    return {
        onCreateConversationThread,
        onDeleteConversationThread,
        onSelectConversationThread,
    }
}
