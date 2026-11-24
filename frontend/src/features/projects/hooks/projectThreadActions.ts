import { useCallback } from 'react'

import {
    deleteConversationValidated,
    fetchProjectConversationListValidated,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/components/app/dialog-controller'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { useStore } from '@/store'
import { requestNavigation } from '@/state/workspaceSlice'
import {
    applySummarySnapshotToCache,
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

type UseProjectThreadActionsArgs = {
    activateConversationThread: (projectPath: string, conversationId: string, source?: string) => void
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
    activateConversationThread,
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

    // New chat opens an empty chat in the project, as an empty project shows one; the first
    // message or setting saves it, so New chat leaves no empty chats behind.
    const onCreateConversationThread = useCallback((projectPath: string) => {
        const open = () => {
            useStore.setState({ projectPagePath: null })
            setPanelError(null)
            updateProjectSessionState(projectPath, { conversationId: null })
            void persistProjectState(projectPath, { active_conversation_id: null, last_accessed_at: new Date().toISOString() })
            useStore.getState().setActiveProjectPath(projectPath)
            // The new chat's composer takes focus once it shows, so typing starts the chat.
            requestAnimationFrame(() => {
                document.querySelector<HTMLElement>('[data-testid="chats-view"] [data-testid="project-ai-conversation-input"]')?.focus()
            })
        }
        if (useStore.getState().projectPagePath) requestNavigation(open)
        else open()
    }, [
        persistProjectState,
        setPanelError,
        updateProjectSessionState,
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
        const environmentId = getActiveEnvironmentId()
        try {
            await deleteConversationValidated(conversationId, projectPath, environmentId)
            // Deleted, or refreshed, after a switch: the chats shown are another environment's, and stay as they are.
            if (getActiveEnvironmentId() !== environmentId) return
            commitConversationCache((current) => removeConversationFromCache({
                ...current,
                summariesByProjectPath: {
                    ...current.summariesByProjectPath,
                    [projectPath]: (current.summariesByProjectPath[projectPath] ?? []).filter((entry) => entry.conversation_id !== conversationId),
                },
            }, conversationId))
            clearHomeConversationSession(conversationId)
            // Keep the local optimistic removal if the follow-up refresh fails.
            const snapshot = await fetchProjectConversationListValidated(projectPath).catch(() => null)
            if (getActiveEnvironmentId() !== environmentId) return
            if (snapshot) commitConversationCache((current) => applySummarySnapshotToCache(current, snapshot, 'http', projectPath))
            const remainingSummaries = useStore.getState().homeConversationCache.summariesByProjectPath[projectPath] ?? []

            if (useStore.getState().projectSessionsByPath[projectPath]?.conversationId === conversationId) {
                const fallbackConversationId = remainingSummaries[0]?.conversation_id || null
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
        persistProjectState,
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
