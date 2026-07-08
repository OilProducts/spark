import type { Dispatch, FormEvent, KeyboardEvent, SetStateAction } from 'react'
import {
    sendConversationTurnValidated,
    type ConversationChatMode,
    type ConversationSnapshotResponse,
    updateConversationSettingsValidated,
} from '@/lib/workspaceClient'
import type { PendingConversationTurnState } from '../model/conversationState'

export type ConversationComposerCommand =
    | {
        kind: 'switch_mode'
        chatMode: ConversationChatMode
    }
    | {
        kind: 'switch_and_send'
        chatMode: ConversationChatMode
        message: string
    }

export function parseConversationComposerCommand(value: string): ConversationComposerCommand | null {
    const trimmed = value.trim()
    const match = trimmed.match(/^\/(plan|chat)(?:\s+([\s\S]*))?$/)
    if (!match) {
        return null
    }
    const chatMode = match[1] === 'plan' ? 'plan' : 'chat'
    const message = (match[2] || '').trim()
    if (!message) {
        return {
            kind: 'switch_mode',
            chatMode,
        }
    }
    return {
        kind: 'switch_and_send',
        chatMode,
        message,
    }
}

type UseConversationComposerArgs = {
    activeProjectPath: string | null
    chatDraft: string
    isChatInputDisabled: boolean
    provider: string
    model: string
    reasoningEffort: string
    ensureConversationId: () => string | null
    getCurrentConversationId: (projectPath: string) => string | null
    getCurrentConversationRevision: (conversationId: string) => number
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source?: string,
        options?: { forceWorkspaceSync?: boolean },
    ) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    setChatDraft: Dispatch<SetStateAction<string>>
    setPanelError: (message: string | null) => void
    setPendingConversationTurn: Dispatch<SetStateAction<PendingConversationTurnState | null>>
}

export function useConversationComposer({
    activeProjectPath,
    chatDraft,
    isChatInputDisabled,
    provider,
    model,
    reasoningEffort,
    ensureConversationId,
    getCurrentConversationId,
    getCurrentConversationRevision,
    applyConversationSnapshot,
    formatErrorMessage,
    setChatDraft,
    setPanelError,
    setPendingConversationTurn,
}: UseConversationComposerArgs) {
    const resetComposer = () => {
        setChatDraft('')
        setPendingConversationTurn(null)
    }

    const onSendChatMessage = async () => {
        if (!activeProjectPath || isChatInputDisabled) {
            return
        }
        const trimmed = chatDraft.trim()
        if (!trimmed) {
            return
        }
        const parsedCommand = parseConversationComposerCommand(trimmed)
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }

        setPanelError(null)
        setChatDraft('')
        if (parsedCommand?.kind === 'switch_mode') {
            try {
                const snapshot = await updateConversationSettingsValidated(conversationId, {
                    project_path: activeProjectPath,
                    chat_mode: parsedCommand.chatMode,
                })
                applyConversationSnapshot(activeProjectPath, snapshot, 'settings-response', {
                    forceWorkspaceSync: true,
                })
            } catch (error) {
                const message = formatErrorMessage(error, 'Unable to switch the project chat mode.')
                setPanelError(message)
            }
            return
        }
        const messageToSend = parsedCommand?.kind === 'switch_and_send' ? parsedCommand.message : trimmed
        const chatMode = parsedCommand?.kind === 'switch_and_send' ? parsedCommand.chatMode : null
        setPendingConversationTurn({
            conversationId,
            afterRevision: getCurrentConversationRevision(conversationId),
        })
        try {
            const snapshot = await sendConversationTurnValidated(conversationId, {
                project_path: activeProjectPath,
                message: messageToSend,
                provider: provider.trim() || 'codex',
                model: model.trim() || null,
                reasoning_effort: reasoningEffort.trim(),
                chat_mode: chatMode,
            })
            setPendingConversationTurn(null)
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            applyConversationSnapshot(activeProjectPath, snapshot, 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to send the project chat turn.')
            setPanelError(message)
        } finally {
            setPendingConversationTurn(null)
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
