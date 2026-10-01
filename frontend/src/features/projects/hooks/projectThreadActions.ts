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
import { requestNavigation } from '@/state/workspaceSlice'
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
    conversationCacheRef: ConversationCacheRef
    setConversationSummaryList: (projectPath: string, summaries: ConversationSummaryResponse[]) => void
    activateConversationThread: (projectPath: string, conversationId: string, source?: string) => void
    applyConversationSnapshot: (projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => unknown
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
    conversationCacheRef,
    setConversationSummaryList,
    activateConversationThread,
    applyConversationSnapshot,
    updateProjectSessionState,
    clearHomeConversationSession,
    setPanelError,
    setPendingDeleteConversationId,
    commitConversationCache,
    persistProjectState,
}: UseProjectThreadActionsArgs) {
    const { confirm } = useDialogController()

    // Each chat carries its project: opening one shows it, in its project.
    // Leaving a project page asks first, so a cancelled leave changes nothing.
    const onSelectConversationThread = useCallback((projectPath: string, conversationId: string) => {
        const open = () => {
            useStore.setState({ projectPagePath: null })
            setPanelError(null)
            activateConversationThread(projectPath, conversationId, 'select-thread')
            useStore.getState().setActiveProjectPath(projectPath)
        }
        if (useStore.getState().projectPagePath) requestNavigation(open)
        else open()
    }, [
        activateConversationThread,
        setPanelError,
    ])

    const onCreateConversationThread = useCallback(async (projectPath: string) => {
        const conversationId = buildProjectConversationId(projectPath)
        const whereYouWere = () => {
            const state = useStore.getState()
            const shownProjectPath = state.activeProjectPath
            return JSON.stringify([shownProjectPath, state.projectPagePath,
                shownProjectPath ? state.projectSessionsByPath[shownProjectPath]?.conversationId ?? null : null])
        }
        const before = whereYouWere()
        setPanelError(null)
        try {
            // Persist first so an empty thread survives reload and can be deleted.
            const snapshot = await updateConversationSettingsValidated(conversationId, {
                project_path: projectPath,
                expected_revision: '0',
            })
            applyConversationSnapshot(projectPath, snapshot, 'create-thread')
            // Only open it if you are still where you were when you asked for it.
            if (whereYouWere() === before) {
                onSelectConversationThread(projectPath, conversationId)
            }
        } catch (error) {
            setPanelError(extractApiErrorMessage(error, 'Unable to create the thread.'))
        }
    }, [
        applyConversationSnapshot,
        onSelectConversationThread,
        setPanelError,
    ])

    const onDeleteConversationThread = useCallback(async (projectPath: string, conversationId: string, title: string) => {
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
            await deleteConversationValidated(conversationId, projectPath)
            commitConversationCache((current) => removeConversationFromCache(current, conversationId))
            clearHomeConversationSession(conversationId)
            const localRemainingSummaries = (
                conversationCacheRef.current.summariesByProjectPath[projectPath] || []
            ).filter((entry) => entry.conversation_id !== conversationId)
            setConversationSummaryList(projectPath, localRemainingSummaries)

            let remainingSummaries = localRemainingSummaries
            try {
                remainingSummaries = await fetchProjectConversationListValidated(projectPath)
                setConversationSummaryList(projectPath, remainingSummaries)
            } catch {
                // Keep the local optimistic removal if the follow-up refresh fails.
            }

            if (useStore.getState().projectSessionsByPath[projectPath]?.conversationId === conversationId) {
                const fallbackConversationId = remainingSummaries[0]?.conversation_id || null
                // The deleted chat's draft must not carry into the fallback, whichever project is shown.
                useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: '', pendingConversationTurn: null })
                updateProjectSessionState(projectPath, {
                    conversationId: fallbackConversationId,
                })
                void persistProjectState(projectPath, {
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
        commitConversationCache,
        conversationCacheRef,
        persistProjectState,
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
