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
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
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
import { homeSessionStart } from '@/state/homeSessionSlice'
import { useStore, whenEnvironmentShown } from '@/store'
import type { PendingConversationTurnState, PendingUserMessage } from '../model/conversationState'
import type { NormalizedConversationRecord } from '../model/projectsHomeState'
import type { ComposerAttachment } from '../model/types'
import { clearSentAttachments, restoreRefusedAttachments } from './useComposerAttachments'

// Prevents duplicate turn submits while a turn waits for an in-flight settings save to commit. A chat
// is one in its environment's chats as shown: the same conversation ID elsewhere, or once a switch away
// and back starts them anew, is another chat.
const awaitingSettingsTurn = new Set<string>()

/**
 * Applies what a refused turn puts back once its environment is shown and `ready`, such as with its project's
 * chats loaded. A switch away first holds it again for that environment's next showing, which starts its chats anew.
 */
function whenShownAndReady(environmentId: string | null, ready: () => boolean, apply: () => void) {
    whenEnvironmentShown(environmentId, () => {
        const session = homeSessionStart()
        const attempt = () => {
            const restarted = homeSessionStart() !== session
            if (!restarted && !ready()) return
            stop()
            if (restarted) whenShownAndReady(environmentId, ready, apply)
            else apply()
        }
        const stop = useStore.subscribe(attempt)
        attempt()
    })
}

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

export function isModelComposerCommand(value: string): boolean {
    const parsed = parseConversationComposerCommand(value)
    return parsed?.kind === 'open_model_picker'
        || parsed?.kind === 'set_model'
        || parsed?.kind === 'set_effort'
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
    /**
     * Awaits any in-flight model settings save so a competing send uses acknowledged settings without
     * 409 conflicts: false when one failed, else the settings revision they acknowledged (true when none ran).
     */
    waitForPendingSettingsSave?: (conversationId: string) => Promise<boolean | string>
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
    const awaitSettingsReady = async (conversationId: string, environmentId: string | null): Promise<boolean | string> => {
        if (!waitForPendingSettingsSave) return true
        const key = `${environmentId ?? 'local'}\n${homeSessionStart()}\n${conversationId}`
        if (awaitingSettingsTurn.has(key)) return false
        awaitingSettingsTurn.add(key)
        try {
            return await waitForPendingSettingsSave(conversationId)
        } finally {
            awaitingSettingsTurn.delete(key)
        }
    }

    // A request goes to the environment it started in, captured before any await; once another
    // environment is shown, its answer changes nothing there.
    const isShown = (environmentId: string | null) => getActiveEnvironmentId() === environmentId
    // Nor, once a switch away and back has started the chats anew, does a send begun before then: it
    // shows in the chats it started in, never in a newer send's pending and sending state.
    const showsHere = () => {
        const environmentId = getActiveEnvironmentId()
        const session = homeSessionStart()
        return () => isShown(environmentId) && homeSessionStart() === session
    }
    // The project's chats show once the projects have loaded with it.
    const projectShown = () => Boolean(activeProjectPath && useStore.getState().projectRegistry[activeProjectPath])

    // The `/plan` and `/chat` commands and the header's mode control take the same path.
    const switchChatMode = async (conversationId: string, chatMode: ConversationChatMode) => {
        if (!activeProjectPath) {
            return
        }
        const environmentId = getActiveEnvironmentId()
        const initialRevision = getConversationRecord(conversationId)?.settingsRevision
        const ready = await awaitSettingsReady(conversationId, environmentId)
        if (!ready) {
            return
        }
        // Shown, the chat's record holds every acknowledged save; after a switch it is gone, and the
        // pending settings save, if one ran, acknowledged the newest revision.
        const expectedRevision = (isShown(environmentId) ? getConversationRecord(conversationId)?.settingsRevision : undefined)
            ?? (typeof ready === 'string' ? ready : initialRevision)
            ?? '0'
        if (isShown(environmentId)) setPanelError(null)
        try {
            // Changed and unchanged settings both answer with the conversation row; the reducer
            // keeps whichever of it and a live update is newer.
            const ack = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                chat_mode: chatMode,
                expected_revision: expectedRevision,
            }, environmentId)
            if (!isShown(environmentId)) return
            applyConversationBatch(activeProjectPath, conversationId, ack.changes, 'mutation', 'settings-response', {
                forceWorkspaceSync: true,
            })
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to switch the project chat mode.')
            if (isShown(environmentId)) setPanelError(message)
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
        const environmentId = getActiveEnvironmentId()
        const live = showsHere()
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
        if (!(await awaitSettingsReady(conversationId, environmentId))) {
            return
        }
        // Switched away while settings saved: the message still goes, but nothing here shows it.
        if (live()) setPanelError(null)
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
            if (!live()) return
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
        // The message shows in the transcript now, so the composer clears now; a failed send puts the
        // text and attachments back ahead of anything added meanwhile, in its environment once that is
        // shown again.
        const currentDraft = () => useStore.getState().homeProjectSessionsByPath[activeProjectPath]?.chatDraft ?? ''
        const setDraft = (draft: string) => {
            if (isShown(environmentId)) useStore.getState().updateHomeProjectSession(activeProjectPath, { chatDraft: draft })
        }
        if (live() && currentDraft() === chatDraft) {
            setDraft('')
        }
        const accepted = await runTurn(conversationId, environmentId, live, 'Unable to send the project chat turn.', () => sendConversationTurnValidated(conversationId, {
            project_path: activeProjectPath,
            message: messageToSend,
            client_turn_id: clientTurnId,
            ...(attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : {}),
            chat_mode: chatMode,
        }, environmentId), () => {
            // Accepted: clear what was sent, keeping anything attached meanwhile.
            clearSentAttachments(conversationId, sending.map((entry) => entry.localId))
        })
        if (!accepted) {
            updatePendingMessages((messages) => messages.filter((message) => message.id !== clientTurnId))
            // The text and files go back to the chat they were sent in once it, or no chat, is open in its
            // project, which then opens it; another chat opened meanwhile keeps its own draft.
            // ponytail: held for the page's life if that chat never opens again, as when it is deleted.
            const openChat = () => useStore.getState().projectSessionsByPath[activeProjectPath]?.conversationId ?? null
            whenShownAndReady(environmentId, () => projectShown() && [null, conversationId].includes(openChat()), () => {
                if (openChat() === null) useStore.getState().updateProjectSessionState(activeProjectPath, { conversationId })
                setDraft([chatDraft, currentDraft()].filter(Boolean).join('\n\n'))
                if (sending.length > 0) restoreRefusedAttachments(conversationId, sending)
            })
        }
    }

    /** Resends a failed turn's message as a new turn; the draft is left alone. */
    const onRetryTurn = async (turnId: string) => {
        const conversationId = activeProjectPath ? getCurrentConversationId(activeProjectPath) : null
        if (!activeProjectPath || !conversationId || isChatInputDisabled) {
            return
        }
        const environmentId = getActiveEnvironmentId()
        const live = showsHere()
        if (!(await awaitSettingsReady(conversationId, environmentId))) {
            return
        }
        if (live()) setPanelError(null)
        await runTurn(conversationId, environmentId, live, 'Unable to retry the project chat turn.', () => retryConversationTurnValidated(conversationId, turnId, activeProjectPath, environmentId))
    }

    /** Starts a turn through `start`, tracking it as running until the server answers. */
    const runTurn = async (
        conversationId: string,
        environmentId: string | null,
        live: () => boolean,
        failure: string,
        start: () => Promise<ConversationAcceptance>,
        onAccepted?: () => void,
    ): Promise<boolean> => {
        if (!activeProjectPath) {
            return false
        }
        // A refused turn's error shows in its environment's chats, now or once they show again.
        const report = (error: unknown) => {
            const message = formatErrorMessage(error, failure)
            whenShownAndReady(environmentId, projectShown, () => setPanelError(message))
            return false
        }
        // Nothing here shows a turn from chats left since it began, even ones of this environment shown
        // again: its answer never ends a newer send's pending and sending state, nor merges into them.
        if (!live()) {
            return start().then(() => true, report)
        }
        const afterRevision = getConversationRecord(conversationId)?.latestRevision ?? '0'
        setPendingConversationTurn({ conversationId, afterRevision })
        // The project's pending turn is its open chat's: once another chat opens, it is that chat's send.
        const endPending = () => {
            if (getCurrentConversationId(activeProjectPath) === conversationId) setPendingConversationTurn(null)
        }
        // The activity bar follows the turn even after the chat is left.
        const { setSendingChat } = useStore.getState()
        setSendingChat(conversationId, true)
        try {
            const acceptance = await start()
            if (!live()) return true
            onAccepted?.()
            endPending()
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            // The accepted user turn carries the sent client_turn_id, so it replaces the pending
            // message in place; a resend of that id returns the same acceptance and rows.
            applyConversationBatch(activeProjectPath, conversationId, acceptance.changes, 'mutation', 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
            return true
        } catch (error) {
            return report(error)
        } finally {
            if (live()) {
                endPending()
                setSendingChat(conversationId, false)
            }
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
