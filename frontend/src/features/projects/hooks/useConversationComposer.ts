import type { Dispatch, SetStateAction } from 'react'
import {
    buildModelCatalog,
    EMPTY_MODEL_SETTINGS,
    matchAvailableModel,
    recordRecentModel,
    resolveEffortCommand,
    selectModelSettings,
    type ModelDiscoveryState,
} from '@/components/model-chooser/modelSelection'
import type { ModelSettings } from '@/lib/api/settingsApi'
import type { LlmProfileMetadata } from '@/lib/llmSuggestions'
import {
    retryConversationTurnValidated,
    sendConversationTurnValidated,
    type ConversationAcceptance,
    type ConversationBatch,
    type ConversationChatMode,
    updateConversationSettingsValidated,
} from '@/lib/workspaceClient'
import { randomHexId } from '@/lib/utils'
import { useStore } from '@/store'
import type { PendingConversationTurnState, PendingUserMessage } from '../model/conversationState'
import type { NormalizedConversationRecord } from '../model/projectsHomeState'
import type { ComposerAttachment } from '../model/types'
import { clearSentAttachments } from './useComposerAttachments'

// Prevents duplicate turn submits while a turn waits for an in-flight settings save to commit.
const awaitingSettingsTurn = new Set<string>()

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
    | {
        kind: 'open_model_picker'
    }
    | {
        kind: 'set_model'
        query: string
    }
    | {
        kind: 'set_effort'
        effort: string
    }

export function parseConversationComposerCommand(value: string): ConversationComposerCommand | null {
    const trimmed = value.trim()
    const match = trimmed.match(/^\/(plan|chat|model|effort)(?:\s+([\s\S]*))?$/)
    if (!match) {
        return null
    }
    const command = match[1]
    const arg = (match[2] || '').trim()
    if (command === 'model') {
        return arg ? { kind: 'set_model', query: arg } : { kind: 'open_model_picker' }
    }
    if (command === 'effort') {
        return { kind: 'set_effort', effort: arg }
    }
    const chatMode = command === 'plan' ? 'plan' : 'chat'
    if (!arg) {
        return {
            kind: 'switch_mode',
            chatMode,
        }
    }
    return {
        kind: 'switch_and_send',
        chatMode,
        message: arg,
    }
}

type UseConversationComposerArgs = {
    activeProjectPath: string | null
    chatDraft: string
    attachments: ComposerAttachment[]
    isUploadBlocking: boolean
    isChatInputDisabled: boolean
    /** Locks model/effort commands while hydrating, starting a turn, or when the chat is unavailable. */
    isModelSettingsLocked?: boolean
    modelSettings?: ModelSettings
    inheritedModelSettings?: ModelSettings
    modelDiscovery?: ModelDiscoveryState | null
    llmProfiles?: LlmProfileMetadata[]
    onModelSettingsChange?: (value: ModelSettings) => void
    onOpenModelPicker?: () => void
    /** Awaits any in-flight model settings save so a competing send uses acknowledged settings without 409 conflicts. */
    waitForPendingSettingsSave?: (conversationId: string) => Promise<boolean>
    /** Set when the active harness does not support Plan mode; blocks `/plan` before switching. */
    planModeUnsupportedReason?: string | null
    ensureConversationId: () => string | null
    getCurrentConversationId: (projectPath: string) => string | null
    getConversationRecord: (conversationId: string) => NormalizedConversationRecord | undefined
    /** Merges a request's own result rows; they never move the live cursor. */
    applyConversationBatch: (
        projectPath: string,
        conversationId: string,
        batch: ConversationBatch,
        kind: 'live' | 'mutation',
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
    attachments,
    isUploadBlocking,
    isChatInputDisabled,
    isModelSettingsLocked = false,
    modelSettings = EMPTY_MODEL_SETTINGS,
    inheritedModelSettings,
    modelDiscovery = null,
    llmProfiles = [],
    onModelSettingsChange,
    onOpenModelPicker,
    waitForPendingSettingsSave,
    planModeUnsupportedReason = null,
    ensureConversationId,
    getCurrentConversationId,
    getConversationRecord,
    applyConversationBatch,
    formatErrorMessage,
    setChatDraft,
    setPanelError,
    setPendingConversationTurn,
}: UseConversationComposerArgs) {
    const awaitSettingsReady = async (conversationId: string): Promise<boolean> => {
        if (!waitForPendingSettingsSave) return true
        if (awaitingSettingsTurn.has(conversationId)) return false
        awaitingSettingsTurn.add(conversationId)
        try {
            return await waitForPendingSettingsSave(conversationId)
        } finally {
            awaitingSettingsTurn.delete(conversationId)
        }
    }

    // The `/plan` and `/chat` commands and the header's mode control take the same path.
    const switchChatMode = async (conversationId: string, chatMode: ConversationChatMode) => {
        if (!activeProjectPath) {
            return
        }
        if (!(await awaitSettingsReady(conversationId))) {
            return
        }
        setPanelError(null)
        try {
            // Changed and unchanged settings both answer with the conversation row; the reducer
            // keeps whichever of it and a live update is newer.
            const ack = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                chat_mode: chatMode,
                expected_revision: getConversationRecord(conversationId)?.settingsRevision ?? '0',
            })
            applyConversationBatch(activeProjectPath, conversationId, ack.changes, 'mutation', 'settings-response', {
                forceWorkspaceSync: true,
            })
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to switch the project chat mode.')
            setPanelError(message)
        }
    }

    const onSwitchChatMode = async (chatMode: ConversationChatMode) => {
        if (!activeProjectPath || isChatInputDisabled) {
            return
        }
        const conversationId = ensureConversationId()
        if (conversationId) {
            await switchChatMode(conversationId, chatMode)
        }
    }

    const onSendChatMessage = async () => {
        if (!activeProjectPath) {
            return
        }
        const trimmed = chatDraft.trim()
        const sending = attachments.filter((entry) => entry.attachment !== null)
        if (!trimmed && sending.length === 0) {
            return
        }
        const parsedCommand = parseConversationComposerCommand(trimmed)
        if (
            parsedCommand?.kind === 'open_model_picker'
            || parsedCommand?.kind === 'set_model'
            || parsedCommand?.kind === 'set_effort'
        ) {
            if (isModelSettingsLocked) {
                setPanelError('Model settings cannot be changed right now.')
                return
            }
            if (parsedCommand.kind === 'open_model_picker') {
                setPanelError(null)
                setChatDraft('')
                onOpenModelPicker?.()
                return
            }
            if (parsedCommand.kind === 'set_model') {
                const { rows } = buildModelCatalog(llmProfiles, modelDiscovery, false)
                const matched = matchAvailableModel(parsedCommand.query, rows, modelDiscovery, llmProfiles)
                if (!matched.ok) {
                    setPanelError(matched.error)
                    return
                }
                const next = {
                    ...selectModelSettings(modelSettings, matched.choice, llmProfiles, modelDiscovery),
                    reasoning_effort: modelSettings.reasoning_effort,
                }
                recordRecentModel({
                    provider: matched.choice.provider,
                    llm_profile: matched.choice.llm_profile,
                    model: matched.choice.model,
                })
                setPanelError(null)
                setChatDraft('')
                onModelSettingsChange?.(next)
                return
            }
            const resolved = resolveEffortCommand(
                parsedCommand.effort,
                modelSettings,
                inheritedModelSettings,
                llmProfiles,
                modelDiscovery,
            )
            if (!resolved.ok) {
                setPanelError(resolved.error)
                return
            }
            setPanelError(null)
            setChatDraft('')
            onModelSettingsChange?.(resolved.next)
            return
        }
        if (isChatInputDisabled || isUploadBlocking) {
            return
        }
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }

        if (parsedCommand?.chatMode === 'plan' && planModeUnsupportedReason) {
            setPanelError(planModeUnsupportedReason)
            return
        }
        if (parsedCommand?.kind === 'switch_mode' && sending.length === 0) {
            setChatDraft('')
            await switchChatMode(conversationId, parsedCommand.chatMode)
            return
        }
        if (!(await awaitSettingsReady(conversationId))) {
            return
        }
        setPanelError(null)
        const messageToSend = parsedCommand?.kind === 'switch_and_send'
            ? parsedCommand.message
            : parsedCommand?.kind === 'switch_mode' ? '' : trimmed
        const chatMode = parsedCommand?.chatMode ?? null
        const sentAttachments = sending.map((entry) => entry.attachment!)
        const attachmentIds = sentAttachments.map((attachment) => attachment.id)
        // The server records the message under this id, so the pending entry is
        // replaced in place and a resent request cannot add a second turn.
        const clientTurnId = `turn-${randomHexId()}`
        const updatePendingMessages = (update: (messages: PendingUserMessage[]) => PendingUserMessage[]) => {
            const store = useStore.getState()
            store.updateHomeConversationSession(conversationId, {
                pendingMessages: update(store.homeConversationSessionsById[conversationId]?.pendingMessages ?? []),
            })
        }
        updatePendingMessages((messages) => [...messages, {
            id: clientTurnId,
            kind: 'message',
            role: 'user',
            content: messageToSend,
            timestamp: new Date().toISOString(),
            status: 'pending',
            ...(sentAttachments.length > 0 ? { attachments: sentAttachments } : {}),
        }])
        // The message shows in the transcript now, so the composer clears now;
        // a failed send puts the text back ahead of anything typed meanwhile.
        const currentDraft = () => useStore.getState().homeProjectSessionsByPath[activeProjectPath]?.chatDraft ?? ''
        const setDraft = (draft: string) => useStore.getState().updateHomeProjectSession(activeProjectPath, { chatDraft: draft })
        if (currentDraft() === chatDraft) {
            setDraft('')
        }
        const accepted = await runTurn(conversationId, 'Unable to send the project chat turn.', () => sendConversationTurnValidated(conversationId, {
            project_path: activeProjectPath,
            message: messageToSend,
            client_turn_id: clientTurnId,
            ...(attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : {}),
            chat_mode: chatMode,
        }), () => {
            // Accepted: clear what was sent, keeping anything attached meanwhile.
            clearSentAttachments(conversationId, sending.map((entry) => entry.localId))
        })
        if (!accepted) {
            updatePendingMessages((messages) => messages.filter((message) => message.id !== clientTurnId))
            setDraft([chatDraft, currentDraft()].filter(Boolean).join('\n\n'))
        }
    }

    /** Resends a failed turn's message as a new turn; the draft is left alone. */
    const onRetryTurn = async (turnId: string) => {
        const conversationId = activeProjectPath ? getCurrentConversationId(activeProjectPath) : null
        if (!activeProjectPath || !conversationId || isChatInputDisabled) {
            return
        }
        if (!(await awaitSettingsReady(conversationId))) {
            return
        }
        setPanelError(null)
        await runTurn(conversationId, 'Unable to retry the project chat turn.', () => retryConversationTurnValidated(conversationId, turnId, activeProjectPath))
    }

    /** Starts a turn through `start`, tracking it as running until the server answers. */
    const runTurn = async (
        conversationId: string,
        failure: string,
        start: () => Promise<ConversationAcceptance>,
        onAccepted?: () => void,
    ): Promise<boolean> => {
        if (!activeProjectPath) {
            return false
        }
        const afterRevision = getConversationRecord(conversationId)?.latestRevision ?? '0'
        setPendingConversationTurn({ conversationId, afterRevision })
        // The activity bar follows the turn even after the chat is left.
        const { setSendingChat } = useStore.getState()
        setSendingChat(conversationId, true)
        try {
            const acceptance = await start()
            onAccepted?.()
            setPendingConversationTurn(null)
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            // The accepted user turn carries the sent client_turn_id, so it replaces the pending
            // message in place; a resend of that id returns the same acceptance and rows.
            applyConversationBatch(activeProjectPath, conversationId, acceptance.changes, 'mutation', 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
            return true
        } catch (error) {
            const message = formatErrorMessage(error, failure)
            setPanelError(message)
            return false
        } finally {
            setPendingConversationTurn(null)
            setSendingChat(conversationId, false)
        }
    }

    const onChatComposerSubmit = () => {
        void onSendChatMessage()
    }

    return {
        onChatComposerSubmit,
        onRetryTurn,
        onSwitchChatMode,
    }
}
