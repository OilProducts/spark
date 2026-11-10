import { useInheritedModelSettings } from '@/components/model-chooser/useInheritedModelSettings'
import { useModelOptions } from '@/components/model-chooser/useModelOptions'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { ModelSettings } from '@/lib/api/settingsApi'
import {
    ConversationResetError,
    compareRevisions,
    conversationAttachmentUrl,
    fetchConversationPageValidated,
    submitConversationRequestUserInputValidated,
    interruptConversationTurnValidated,
    updateConversationSettingsValidated,
    type ConversationChatMode,
} from '@/lib/workspaceClient'
import { useScrollFollow } from './useScrollFollow'
import { useConversationComposer } from './useConversationComposer'
import { useComposerAttachments } from './useComposerAttachments'
import { useConversationReviews } from './useConversationReviews'
import { useProjectConversationCache } from './useProjectConversationCache'
import { useProjectsHomeInteractionState } from './useProjectsHomeInteractionState'
import { usePersistProjectState } from './usePersistProjectState'
import { useProjectThreadActions } from './projectThreadActions'
import { debugProjectChat } from '../model/projectChatDebug'
import { isThinkingEntry } from '../model/conversationTimeline'
import { buildProjectsHomeViewModel, isPendingMessageRecorded } from '../model/projectsHomeViewModel'
import { projectLabel } from '../model/projectChoices'
import { latestContextUsage } from '../components/ContextMeter'
import {
    buildProjectConversationId,
    extractApiErrorMessage,
    formatConversationAgeShort,
    formatConversationTimestamp,
} from '../model/projectsHomeState'

const INHERITED_MODEL_SETTINGS: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }

const getConversationRecord = (conversationId: string) => useStore.getState().homeConversationCache.conversationsById[conversationId]

export function useProjectsHomeController() {
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const homeThreadSummariesStatusByProjectPath = useStore((state) => state.homeThreadSummariesStatusByProjectPath)
    const projectRegistry = useStore((state) => state.projectRegistry)
    const clearHomeConversationSession = useStore((state) => state.clearHomeConversationSession)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const projectGitMetadata = useStore((state) => state.homeProjectGitMetadataByPath)
    const model = useStore((state) => state.model)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setRunsSelectedRunId = useStore((state) => state.setRunsSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const inheritedModelSettings = useInheritedModelSettings(activeProjectPath)
    const {
        applyConversationBatch,
        applyConversationPage,
        commitConversationCache,
        conversationCache,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        updateProjectSessionState,
    })
    const activeConversationRecord = activeConversationId
        ? conversationCache.conversationsById[activeConversationId] || null
        : null
    // Until the snapshot loads, the thread list already knows the chat's title and age.
    const activeConversationSummary = activeProjectPath && activeConversationId
        ? conversationCache.summariesByProjectPath[activeProjectPath]?.find((entry) => entry.conversation_id === activeConversationId) ?? null
        : null
    const isConversationUnavailable = Boolean(activeConversationId && conversationCache.unavailableConversationIds?.[activeConversationId])
    const isConversationHistoryLoading = Boolean(activeConversationId) && activeConversationRecord === null && !isConversationUnavailable
    const {
        chatDraft,
        expandedThinkingEntries,
        expandedToolCalls,
        panelError,
        pendingConversationTurn,
        pendingDeleteConversationId,
        pendingMessages,
        setChatDraft,
        setPanelError,
        setPendingConversationTurn,
        setPendingDeleteConversationId,
        toggleThinkingEntryExpanded,
        toggleToolCallExpanded,
    } = useProjectsHomeInteractionState({
        activeConversationId,
        activeProjectPath,
    })
    const [requestUserInputActionError, setRequestUserInputActionError] = useState<string | null>(null)
    const [submittingRequestUserInputIds, setSubmittingRequestUserInputIds] = useState<Record<string, boolean>>({})
    // The transcript's "Show thinking" toggle lives in the sheet header, so it stays in view.
    const showThinking = useStore((state) => state.preferredShowThinking)
    const setShowThinking = useStore((state) => state.setPreferredShowThinking)
    // Settings edits round-trip through the server before the conversation
    // snapshot updates; showing the requested values while the save is in
    // flight keeps the selects consistent (no stale model from the previous
    // provider shown under a freshly picked provider).
    const [chatSettingsDrafts, setChatSettingsDrafts] = useState<Record<string, ModelSettings>>({})
    const pendingChatSettings = activeConversationId ? chatSettingsDrafts[activeConversationId] ?? null : null
    const chatSettingsSaves = useRef(new Map<string, { values: ModelSettings | null }>())
    const projectsHomeViewModel = useMemo(() => buildProjectsHomeViewModel({
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        pendingConversationTurn,
        pendingMessages,
        projectGitMetadata,
        uiDefaults,
    }), [
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        activeProjectScope,
        pendingConversationTurn,
        pendingMessages,
        projectGitMetadata,
        uiDefaults,
    ])
    const activeConversationHistory = projectsHomeViewModel.activeConversationHistory
    const {
        activeChatMode,
        activeProjectChatProvider: storedChatProvider,
        activeProjectChatModel: storedChatModel,
        activeProjectChatReasoningEffort: storedChatReasoningEffort,
        activeFlowLaunchesById,
        activeFlowRunRequestsById,
        activeProposedPlansById,
        activeRunRecoveriesById,
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
        isConversationTurnStarting,
        latestFlowLaunchId,
        latestFlowRunRequestId,
    } = projectsHomeViewModel
    const effectiveModelSettings = activeConversationRecord?.model_settings_view?.effective
    const activeProjectChatProvider = pendingChatSettings ? pendingChatSettings.llm_profile || pendingChatSettings.provider || 'codex' : effectiveModelSettings?.llm_profile || storedChatProvider
    const currentModelSettings = useMemo<ModelSettings>(() => pendingChatSettings ?? effectiveModelSettings ?? {
        provider: !activeConversationRecord && uiDefaults.llm_profile ? null : storedChatProvider || 'codex',
        llm_profile: !activeConversationRecord ? uiDefaults.llm_profile || null : null,
        model: storedChatModel || null,
        reasoning_effort: storedChatReasoningEffort || null,
    }, [pendingChatSettings, effectiveModelSettings, activeConversationRecord, uiDefaults.llm_profile, storedChatProvider, storedChatModel, storedChatReasoningEffort])
    // A saved chat's revision is unknown until its snapshot loads, so its settings can't be edited yet.
    const isConversationHydrating = Boolean(activeProjectPath && activeConversationId && !activeConversationRecord
        && conversationCache.summariesByProjectPath[activeProjectPath]?.some((entry) => entry.conversation_id === activeConversationId))
    const editableModelSettings = pendingChatSettings ?? activeConversationRecord?.model_settings_view?.stored ?? INHERITED_MODEL_SETTINGS
    const discovery = useModelOptions(activeProjectPath)
    const activeProjectChatModelsResponse = discovery?.payload
    const activeProjectChatModels = activeProjectChatModelsResponse?.models || []
    const isCodexProvider = (activeProjectChatProvider || 'codex') === 'codex'
    const codexModels = activeProjectChatModels.filter((model) => model.provider === 'codex')
    // Keep the resolved Codex default separate from the editable model settings.
    const activeProjectChatModel = (pendingChatSettings ? pendingChatSettings.model || '' : storedChatModel)
        || (isCodexProvider ? codexModels.find((model) => model.is_default)?.id ?? '' : '')
    const isChatModelSelectable = !isCodexProvider || codexModels.some((model) => model.id === activeProjectChatModel)
    const isChatModelReady = !isCodexProvider || Boolean(activeProjectChatModelsResponse) && isChatModelSelectable
    const chatModelAvailabilityMessage = isCodexProvider
        ? activeProjectChatModelsResponse?.providers.codex.status === 'unavailable'
            ? activeProjectChatModelsResponse.providers.codex.error || 'Codex model discovery failed.'
            : activeProjectChatModelsResponse?.providers.codex.status === 'available' && codexModels.length === 0
                ? 'No Codex models are available.'
                : activeProjectChatModelsResponse && !isChatModelSelectable
                    ? 'The saved Codex model is no longer available. Select another model.'
                    : null
        : null
    const isChatSubmissionDisabled = isConversationTurnStarting || !isChatModelReady
    const follow = useScrollFollow(activeConversationId, hasRenderableConversationHistory)

    const activateConversationThread = useCallback((projectPath: string, conversationId: string, source = 'unknown') => {
        debugProjectChat('activate conversation thread', {
            source,
            projectPath,
            conversationId,
        })
        updateProjectSessionState(projectPath, { conversationId })
        void persistProjectState(projectPath, {
            active_conversation_id: conversationId,
            last_accessed_at: new Date().toISOString(),
        })
    }, [persistProjectState, updateProjectSessionState])

    const ensureConversationId = useCallback(() => {
        if (!activeProjectPath) {
            return null
        }
        if (activeConversationId) {
            return activeConversationId
        }
        const conversationId = buildProjectConversationId(activeProjectPath)
        activateConversationThread(activeProjectPath, conversationId, 'ensure-conversation')
        return conversationId
    }, [activeConversationId, activeProjectPath, activateConversationThread])

    useEffect(() => {
        setRequestUserInputActionError(null)
        setSubmittingRequestUserInputIds({})
    }, [activeConversationId])

    useEffect(() => {
        if (!pendingConversationTurn || !activeConversationRecord) {
            return
        }
        if (
            pendingConversationTurn.conversationId === activeConversationRecord.conversation_id
            && compareRevisions(activeConversationRecord.latestRevision, pendingConversationTurn.afterRevision) > 0
        ) {
            setPendingConversationTurn(null)
        }
    }, [activeConversationRecord, pendingConversationTurn, setPendingConversationTurn])

    // A sent message stays pending until the server records it under its id.
    useEffect(() => {
        if (!activeConversationRecord || !pendingMessages.some((message) => isPendingMessageRecorded(activeConversationRecord, message.id))) {
            return
        }
        const conversationId = activeConversationRecord.conversation_id
        const store = useStore.getState()
        store.updateHomeConversationSession(conversationId, {
            pendingMessages: (store.homeConversationSessionsById[conversationId]?.pendingMessages ?? [])
                .filter((message) => !isPendingMessageRecorded(activeConversationRecord, message.id)),
        })
    }, [activeConversationRecord, pendingMessages])

    const attachmentUrl = useMemo(() => (
        activeConversationId && activeProjectPath
            ? (attachmentId: string) => conversationAttachmentUrl(activeConversationId, attachmentId, activeProjectPath)
            : undefined
    ), [activeConversationId, activeProjectPath])
    const composerAttachments = useComposerAttachments({
        activeConversationId,
        activeProjectPath,
        ensureConversationId,
        setPanelError,
    })

    const {
        onChatComposerSubmit,
        onRetryTurn,
        onSwitchChatMode,
    } = useConversationComposer({
        activeProjectPath,
        chatDraft,
        attachments: composerAttachments.attachments,
        isUploadBlocking: composerAttachments.isUploadBlocking,
        isChatInputDisabled: isChatSubmissionDisabled || isConversationUnavailable,
        ensureConversationId,
        getCurrentConversationId: (projectPath) => (
            useStore.getState().projectSessionsByPath[projectPath]?.conversationId ?? null
        ),
        getConversationRecord,
        applyConversationBatch,
        formatErrorMessage: extractApiErrorMessage,
        setChatDraft,
        setPanelError,
        setPendingConversationTurn,
    })

    const persistChatSettings = useCallback(async (values: ModelSettings | null) => {
        if (!activeProjectPath) return
        const conversationId = ensureConversationId()
        if (!conversationId) return
        setPanelError(null)
        setChatSettingsDrafts((drafts) => ({ ...drafts, [conversationId]: values ?? currentModelSettings }))
        const running = chatSettingsSaves.current.get(conversationId)
        if (running) {
            running.values = values
            return
        }
        const save = { values }
        chatSettingsSaves.current.set(conversationId, save)
        let revision = getConversationRecord(conversationId)?.settingsRevision ?? '0'
        try {
            // Coalesce edits while a save is in flight, then use its acknowledged revision.
            // A conflict stops the queue and retains the draft; never retry over another writer.
            while (true) {
                const saving = save.values
                const ack = await updateConversationSettingsValidated(conversationId, {
                    project_path: activeProjectPath,
                    expected_revision: revision,
                    model_settings: saving,
                })
                revision = ack.revision
                applyConversationBatch(activeProjectPath, conversationId, ack.changes, 'mutation', 'chat-settings-response')
                if (save.values === saving) break
            }
            setChatSettingsDrafts((drafts) => {
                const next = { ...drafts }
                delete next[conversationId]
                return next
            })
        } catch (error) {
            setPanelError(extractApiErrorMessage(error, 'Unable to update the project chat settings.'))
        } finally {
            chatSettingsSaves.current.delete(conversationId)
        }
    }, [activeProjectPath, applyConversationBatch, ensureConversationId, setPanelError, currentModelSettings])

    const onUseModelDefaults = () => {
        if (!activeProjectPath || !activeConversationId || pendingChatSettings) return
        void persistChatSettings(null)
    }

    const {
        onCreateConversationThread,
        onDeleteConversationThread,
        onSelectConversationThread,
    } = useProjectThreadActions({
        activateConversationThread,
        applyConversationBatch,
        updateProjectSessionState,
        clearHomeConversationSession,
        setPanelError,
        setPendingDeleteConversationId,
        commitConversationCache,
        persistProjectState,
    })

    const {
        onReviewFlowRunRequest,
        onReviewProposedPlan,
        pendingFlowRunRequestId,
        pendingProposedPlanId,
    } = useConversationReviews({
        activeConversationId,
        activeProjectPath,
        applyConversationBatch,
        formatErrorMessage: extractApiErrorMessage,
        model,
        setPanelError,
    })

    const onOpenFlowRun = useCallback((request: { run_id?: string | null; flow_name: string }) => {
        if (!request.run_id) {
            return
        }
        setRunsSelectedRunId(request.run_id)
        setViewMode('runs')
    }, [setRunsSelectedRunId, setViewMode])

    const onStopTurn = useCallback(async () => {
        if (!activeConversationId || !activeProjectPath) return
        try {
            const interrupted = await interruptConversationTurnValidated(activeConversationId, activeProjectPath)
            if (!interrupted) setPanelError('This turn is no longer running or does not support Stop.')
        } catch (error) {
            setPanelError(extractApiErrorMessage(error, 'Unable to stop the turn.'))
        }
    }, [activeConversationId, activeProjectPath, setPanelError])

    const onSubmitRequestUserInput = useCallback(async (requestId: string, answers: Record<string, string>) => {
        if (!activeConversationId || !activeProjectPath) {
            return
        }
        setRequestUserInputActionError(null)
        setSubmittingRequestUserInputIds((current) => ({
            ...current,
            [requestId]: true,
        }))
        try {
            const ack = await submitConversationRequestUserInputValidated(
                activeConversationId,
                requestId,
                {
                    project_path: activeProjectPath,
                    answers,
                },
            )
            applyConversationBatch(activeProjectPath, activeConversationId, ack.changes, 'mutation', 'request-user-input-answer')
        } catch (error) {
            const message = extractApiErrorMessage(error, 'Unable to submit the requested input.')
            setRequestUserInputActionError(message)
        } finally {
            setSubmittingRequestUserInputIds((current) => {
                const next = { ...current }
                delete next[requestId]
                return next
            })
        }
    }, [activeConversationId, activeProjectPath, applyConversationBatch])

    // Older pages load on request; the newest window loads with the chat.
    const [olderMessages, setOlderMessages] = useState<{ conversationId: string; loading: boolean; error: string | null } | null>(null)
    const activeOlderMessages = olderMessages?.conversationId === activeConversationId ? olderMessages : null
    const onLoadOlderMessages = useCallback(async () => {
        const record = activeConversationId ? getConversationRecord(activeConversationId) : undefined
        const cursor = record?.olderCursor
        if (!activeProjectPath || !record || !cursor) return
        const conversationId = record.conversation_id
        setOlderMessages({ conversationId, loading: true, error: null })
        // A newer window, a reset or a generation change replaces the window this request was
        // issued for, and its response is then stale for good, even if the cursor comes back.
        const { windowEpoch } = record
        const isCurrentWindow = () => {
            const current = getConversationRecord(conversationId)
            return current?.windowEpoch === windowEpoch && current.olderCursor === cursor && !current.resetting
        }
        try {
            try {
                const page = await fetchConversationPageValidated(conversationId, activeProjectPath, cursor)
                if (isCurrentWindow()) {
                    applyConversationPage(activeProjectPath, page, 'older', 'older-page')
                }
            } catch (error) {
                if (!(error instanceof ConversationResetError)) throw error
                // Turns moved or the store changed under the cursor: start again from the newest window.
                const page = await fetchConversationPageValidated(conversationId, activeProjectPath)
                if (isCurrentWindow()) {
                    applyConversationPage(activeProjectPath, page, 'window', 'older-page-reset')
                }
            }
            setOlderMessages(null)
        } catch (error) {
            setOlderMessages({ conversationId, loading: false, error: extractApiErrorMessage(error, 'Unable to load older messages.') })
        }
    }, [activeConversationId, activeProjectPath, applyConversationPage])

    return {
        isNarrowViewport,
        historyProps: {
            activeConversationId,
            activeProjectPath,
            attachmentUrl,
            isConversationHistoryLoading,
            hasRenderableConversationHistory,
            activeConversationHistory,
            activeFlowRunRequestsById,
            activeFlowLaunchesById,
            activeProposedPlansById,
            activeRunRecoveriesById,
            latestFlowRunRequestId,
            latestFlowLaunchId,
            expandedToolCalls,
            expandedThinkingEntries,
            showThinking,
            pendingFlowRunRequestId,
            pendingProposedPlanId,
            requestUserInputActionError,
            submittingRequestUserInputIds,
            formatConversationTimestamp,
            onSubmitRequestUserInput,
            onToggleToolCallExpanded: toggleToolCallExpanded,
            onToggleThinkingEntryExpanded: toggleThinkingEntryExpanded,
            onReviewFlowRunRequest,
            onReviewProposedPlan,
            onOpenFlowRun,
            onRetryTurn,
            isConversationUnavailable,
            hasOlderMessages: Boolean(activeConversationRecord?.olderCursor),
            isLoadingOlderMessages: activeOlderMessages?.loading === true,
            olderMessagesError: activeOlderMessages?.error ?? null,
            onLoadOlderMessages,
        },
        sidebarProps: {
            isNarrowViewport,
            activeProjectPath,
            activeConversationId,
            conversationSummariesByProjectPath: conversationCache.summariesByProjectPath,
            conversationSummariesStatusByProjectPath: homeThreadSummariesStatusByProjectPath,
            pendingDeleteConversationId,
            onCreateConversationThread,
            onSelectConversationThread,
            onDeleteConversationThread,
            formatConversationAgeShort,
        },
        surfaceProps: {
            activeProjectLabel: activeProjectPath ? projectLabel(projectRegistry, activeProjectPath) : null,
            activeProjectPath,
            activeConversationTitle: activeConversationRecord?.title ?? activeConversationSummary?.title ?? null,
            activeConversationHandle: activeConversationRecord?.conversation_handle || activeConversationSummary?.conversation_handle || null,
            activeConversationUpdatedAt: activeConversationRecord?.updated_at ?? activeConversationSummary?.updated_at ?? null,
            activeChatMode,
            contextUsage: latestContextUsage((activeConversationRecord?.orderedTurnIds ?? []).map((turnId) => activeConversationRecord?.turnsById[turnId])),
            onChatModeChange: (mode: ConversationChatMode) => { void onSwitchChatMode(mode) },
            hasThinking: activeConversationHistory.some((entry) => isThinkingEntry(entry) && entry.content.trim().length > 0),
            showThinking,
            onShowThinkingChange: setShowThinking,
            // The picker edits what this conversation stores; effective settings would hide inheritance.
            modelSettings: editableModelSettings,
            // While nothing is stored, the conversation's effective settings are what it inherits.
            inheritedModelSettings: editableModelSettings === INHERITED_MODEL_SETTINGS ? currentModelSettings : inheritedModelSettings,
            defaultModel: isCodexProvider && !currentModelSettings.model ? activeProjectChatModel : undefined,
            onModelSettingsChange: (value: ModelSettings) => { void persistChatSettings(value) },
            isModelSettingsLoading: isConversationHydrating,
            chatModelAvailabilityMessage,
            hasRenderableConversationHistory,
            follow,
            isNarrowViewport,
            chatDraft,
            attachments: composerAttachments.attachments,
            onAddAttachments: composerAttachments.addFiles,
            onRetryAttachment: composerAttachments.retry,
            onRemoveAttachment: composerAttachments.remove,
            isAttachmentSendPending: composerAttachments.isSending,
            isUploadBlocking: composerAttachments.isUploadBlocking,
            chatSendButtonLabel,
            // A deleted chat takes no more messages.
            isChatInputDisabled: isChatInputDisabled || isConversationUnavailable,
            isChatSendDisabled: isChatSubmissionDisabled || isConversationUnavailable,
            panelError: panelError || activeConversationRecord?.model_settings_view?.validation_errors?.join(' ') || null,
            onStopTurn: projectsHomeViewModel.hasActiveAssistantTurn ? onStopTurn : undefined,
            onChatComposerSubmit,
            onChatDraftChange: setChatDraft,
            modelSettingsSource: activeConversationRecord?.model_settings_view?.source,
            onUseModelDefaults,
        },
    }
}
