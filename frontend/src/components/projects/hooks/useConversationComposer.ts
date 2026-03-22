import type { Dispatch, FormEvent, KeyboardEvent, SetStateAction } from 'react'
import { sendConversationTurnValidated, type ConversationSnapshotResponse } from '@/lib/workspaceClient'
import type { OptimisticSendState } from '../conversationState'

type UseConversationComposerArgs = {
    activeProjectPath: string | null
    chatDraft: string
    isChatInputDisabled: boolean
    model: string
    ensureConversationId: () => string | null
    getCurrentConversationId: (projectPath: string) => string | null
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source?: string,
        options?: { forceWorkspaceSync?: boolean },
    ) => void
    appendLocalProjectEvent: (message: string) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    setChatDraft: Dispatch<SetStateAction<string>>
    setPanelError: (message: string | null) => void
    setOptimisticSend: Dispatch<SetStateAction<OptimisticSendState | null>>
}

export function useConversationComposer({
    activeProjectPath,
    chatDraft,
    isChatInputDisabled,
    model,
    ensureConversationId,
    getCurrentConversationId,
    applyConversationSnapshot,
    appendLocalProjectEvent,
    formatErrorMessage,
    setChatDraft,
    setPanelError,
    setOptimisticSend,
}: UseConversationComposerArgs) {
    const resetComposer = () => {
        setChatDraft('')
        setOptimisticSend(null)
    }

    const onSendChatMessage = async () => {
        if (!activeProjectPath || isChatInputDisabled) {
            return
        }
        const trimmed = chatDraft.trim()
        if (!trimmed) {
            return
        }
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }
        const optimisticCreatedAt = new Date().toISOString()

        setPanelError(null)
        setChatDraft('')
        setOptimisticSend({
            conversationId,
            message: trimmed,
            createdAt: optimisticCreatedAt,
        })
        try {
            const snapshot = await sendConversationTurnValidated(conversationId, {
                project_path: activeProjectPath,
                message: trimmed,
                model: model.trim() || null,
            })
            setOptimisticSend(null)
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            applyConversationSnapshot(activeProjectPath, snapshot, 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to send the project chat turn.')
            setPanelError(message)
            appendLocalProjectEvent(`Project chat turn failed: ${message}`)
        } finally {
            setOptimisticSend(null)
        }
    }

    const onChatComposerSubmit = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        void onSendChatMessage()
    }

    const onChatComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
        if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault()
            void onSendChatMessage()
        }
    }

    return {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        resetComposer,
    }
}
