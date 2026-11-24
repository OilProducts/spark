import { useCallback, useMemo, useState, type SetStateAction } from 'react'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { useStore } from '@/store'

import type { PendingConversationTurnState, PendingUserMessage } from '../model/conversationState'
import type { ComposerAttachment } from '../model/types'

type UseProjectsHomeInteractionStateArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
}

const EMPTY_PROJECT_SESSION = {
    chatDraft: '',
    panelError: null as string | null,
    pendingConversationTurn: null as PendingConversationTurnState | null,
    pendingDeleteConversationId: null as string | null,
}

const EMPTY_CONVERSATION_SESSION = {
    expandedToolCalls: {} as Record<string, boolean>,
    expandedThinkingEntries: {} as Record<string, boolean>,
    isPinnedToBottom: true,
    scrollTop: null as number | null,
    attachments: [] as ComposerAttachment[],
    pendingMessages: [] as PendingUserMessage[],
}

export function useProjectsHomeInteractionState({
    activeConversationId,
    activeProjectPath,
}: UseProjectsHomeInteractionStateArgs) {
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeConversationSessionsById = useStore((state) => state.homeConversationSessionsById)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const updateHomeConversationSession = useStore((state) => state.updateHomeConversationSession)
    // A chat's draft, error and turn are the environment this mounted in: a reply that lands after a
    // switch writes none of them into the environment shown.
    const [environmentId] = useState(getActiveEnvironmentId)
    const shown = useCallback(() => getActiveEnvironmentId() === environmentId, [environmentId])

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
        if (!activeProjectPath || !shown()) {
            return
        }
        updateHomeProjectSession(activeProjectPath, {
            chatDraft: typeof value === 'function' ? value(projectSession.chatDraft) : value,
        })
    }, [activeProjectPath, projectSession.chatDraft, shown, updateHomeProjectSession])

    const setPendingConversationTurn = useCallback((value: SetStateAction<PendingConversationTurnState | null>) => {
        if (!activeProjectPath || !shown()) {
            return
        }
        updateHomeProjectSession(activeProjectPath, {
            pendingConversationTurn: typeof value === 'function'
                ? value(projectSession.pendingConversationTurn)
                : value,
        })
    }, [activeProjectPath, projectSession.pendingConversationTurn, shown, updateHomeProjectSession])

    const setPanelError = useCallback((value: string | null) => {
        if (!activeProjectPath || !shown()) {
            return
        }
        updateHomeProjectSession(activeProjectPath, { panelError: value })
    }, [activeProjectPath, shown, updateHomeProjectSession])

    const setPendingDeleteConversationId = useCallback((value: string | null) => {
        if (!activeProjectPath || !shown()) {
            return
        }
        updateHomeProjectSession(activeProjectPath, { pendingDeleteConversationId: value })
    }, [activeProjectPath, shown, updateHomeProjectSession])

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
                [entryId]: conversationSession.expandedThinkingEntries[entryId] === false,
            },
        })
    }, [activeConversationId, conversationSession.expandedThinkingEntries, updateHomeConversationSession])

    return {
        chatDraft: projectSession.chatDraft,
        expandedThinkingEntries: conversationSession.expandedThinkingEntries,
        expandedToolCalls: conversationSession.expandedToolCalls,
        panelError: projectSession.panelError,
        pendingConversationTurn: projectSession.pendingConversationTurn,
        pendingDeleteConversationId: projectSession.pendingDeleteConversationId,
        pendingMessages: conversationSession.pendingMessages,
        setChatDraft,
        setPanelError,
        setPendingConversationTurn,
        setPendingDeleteConversationId,
        toggleThinkingEntryExpanded,
        toggleToolCallExpanded,
    }
}
