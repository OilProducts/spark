import { useCallback, type MutableRefObject } from 'react'

import {
    deleteConversationValidated,
    fetchProjectConversationListValidated,
    updateConversationSettingsValidated,
    type ConversationSnapshotResponse,
    type ConversationSummaryResponse,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/components/app/dialog-controller'
import { useStore } from '@/store'
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
    activateConversationThread: (projectPath: string, conversationId: string, source?: string) => void
    applyConversationSnapshot: (projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => unknown
    resetComposer: () => void
    setConversationId: (conversationId: string | null) => void
    updateProjectSessionState: (projectPath: string, patch: Record<string, unknown>) => void
    clearHomeConversationSession: (conversationId: string) => void
    setPanelError: (value: string | null) => void
    setPendingDeleteConversationId: (value: string | null) => void
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
    activateConversationThread,
    applyConversationSnapshot,
    resetComposer,
    setConversationId,
    updateProjectSessionState,
    clearHomeConversationSession,
    setPanelError,
    setPendingDeleteConversationId,
    commitConversationCache,
    persistProjectState,
}: UseProjectThreadActionsArgs) {
    const { confirm } = useDialogController()

    const onCreateConversationThread = useCallback(async () => {
        if (!activeProjectPath) {
            return
        }
        const conversationId = buildProjectConversationId(activeProjectPath)
        setPanelError(null)
        try {
            // Persist first so an empty thread survives reload and can be deleted.
            const snapshot = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                expected_revision: '0',
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'create-thread')
            // Only select it if the user is still in the originating project.
            if (useStore.getState().activeProjectPath !== activeProjectPath) {
                return
            }
            activateConversationThread(activeProjectPath, conversationId, 'create-thread')
        } catch (error) {
            setPanelError(extractApiErrorMessage(error, 'Unable to create the thread.'))
        }
    }, [
        activeProjectPath,
        activateConversationThread,
        applyConversationSnapshot,
        setPanelError,
    ])

    const onSelectConversationThread = useCallback((conversationId: string) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        activateConversationThread(activeProjectPath, conversationId, 'select-thread')
    }, [
        activeProjectPath,
        activateConversationThread,
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
        } finally {
            setPendingDeleteConversationId(null)
        }
    }, [
        activeConversationId,
        activeProjectPath,
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
