import { useCallback, useMemo, type SetStateAction } from 'react'
import { useStore } from '@/store'

import type { PendingSendState } from '../model/conversationState'

type UseProjectsHomeInteractionStateArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
}

const EMPTY_PROJECT_SESSION = {
    chatDraft: '',
    panelError: null as string | null,
    pendingSend: null as PendingSendState | null,
    pendingDeleteConversationId: null as string | null,
    sidebarPrimarySplitRatio: null as number | null,
}

const EMPTY_CONVERSATION_SESSION = {
    expandedToolCalls: {} as Record<string, boolean>,
    expandedThinkingEntries: {} as Record<string, boolean>,
    isPinnedToBottom: true,
    scrollTop: null as number | null,
}

export function useProjectsHomeInteractionState({
    activeConversationId,
    activeProjectPath,
}: UseProjectsHomeInteractionStateArgs) {
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeConversationSessionsById = useStore((state) => state.homeConversationSessionsById)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const updateHomeConversationSession = useStore((state) => state.updateHomeConversationSession)

    const projectSession = useMemo(() => {
        if (!activeProjectPath) {
            return EMPTY_PROJECT_SESSION
        }
        return {
            ...EMPTY_PROJECT_SESSION,
            ...(homeProjectSessionsByPath[activeProjectPath] ?? {}),
        }
    }, [activeProjectPath, homeProjectSessionsByPath])

    const conversationSession = useMemo(() => {
        if (!activeConversationId) {
            return EMPTY_CONVERSATION_SESSION
        }
        return {
            ...EMPTY_CONVERSATION_SESSION,
            ...(homeConversationSessionsById[activeConversationId] ?? {}),
        }
    }, [activeConversationId, homeConversationSessionsById])

    const setChatDraft = useCallback((value: SetStateAction<string>) => {
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, {
            chatDraft: typeof value === 'function' ? value(projectSession.chatDraft) : value,
        })
    }, [activeProjectPath, projectSession.chatDraft, updateHomeProjectSession])

    const setPendingSend = useCallback((value: SetStateAction<PendingSendState | null>) => {
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, {
            pendingSend: typeof value === 'function' ? value(projectSession.pendingSend) : value,
        })
    }, [activeProjectPath, projectSession.pendingSend, updateHomeProjectSession])

    const setPanelError = useCallback((value: string | null) => {
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, { panelError: value })
    }, [activeProjectPath, updateHomeProjectSession])

    const setPendingDeleteConversationId = useCallback((value: string | null) => {
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, { pendingDeleteConversationId: value })
    }, [activeProjectPath, updateHomeProjectSession])

    const toggleToolCallExpanded = useCallback((toolCallId: string) => {
        if (!activeConversationId) {
            return
        }
        updateHomeConversationSession(activeConversationId, {
            expandedToolCalls: {
                ...conversationSession.expandedToolCalls,
                [toolCallId]: !conversationSession.expandedToolCalls[toolCallId],
            },
        })
    }, [activeConversationId, conversationSession.expandedToolCalls, updateHomeConversationSession])

    const toggleThinkingEntryExpanded = useCallback((entryId: string) => {
        if (!activeConversationId) {
            return
        }
        updateHomeConversationSession(activeConversationId, {
            expandedThinkingEntries: {
                ...conversationSession.expandedThinkingEntries,
                [entryId]: !conversationSession.expandedThinkingEntries[entryId],
            },
        })
    }, [activeConversationId, conversationSession.expandedThinkingEntries, updateHomeConversationSession])

    return {
        chatDraft: projectSession.chatDraft,
        expandedThinkingEntries: conversationSession.expandedThinkingEntries,
        expandedToolCalls: conversationSession.expandedToolCalls,
        pendingSend: projectSession.pendingSend,
        panelError: projectSession.panelError,
        pendingDeleteConversationId: projectSession.pendingDeleteConversationId,
        setChatDraft,
        setPendingSend,
        setPanelError,
        setPendingDeleteConversationId,
        toggleThinkingEntryExpanded,
        toggleToolCallExpanded,
    }
}
