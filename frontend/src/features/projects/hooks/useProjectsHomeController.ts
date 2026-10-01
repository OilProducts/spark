import { useInheritedModelSettings } from '@/components/model-chooser/useInheritedModelSettings'
import { useModelOptions } from '@/components/model-chooser/useModelOptions'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { ModelSettings } from '@/lib/api/settingsApi'
import {
    submitConversationRequestUserInputValidated,
    interruptConversationTurnValidated,
    updateConversationSettingsValidated,
} from '@/lib/workspaceClient'
import { useHomeSidebarLayout } from './useHomeSidebarLayout'
import { useConversationComposer } from './useConversationComposer'
import { useConversationReviews } from './useConversationReviews'
import { useProjectConversationCache } from './useProjectConversationCache'
import { useProjectsHomeInteractionState } from './useProjectsHomeInteractionState'
import { usePersistProjectState } from './usePersistProjectState'
import { useProjectThreadActions } from './projectThreadActions'
import { debugProjectChat } from '../model/projectChatDebug'
import { buildProjectsHomeViewModel } from '../model/projectsHomeViewModel'
import { projectLabel } from '../model/projectChoices'
import type { ConversationTimelineEntry } from '../model/types'
import {
    buildProjectConversationId,
    extractApiErrorMessage,
    formatConversationAgeShort,
    formatConversationTimestamp,
} from '../model/projectsHomeState'

function buildConversationHistoryRevisionKey(history: ConversationTimelineEntry[]) {
    const latestEntry = history.at(-1)
    if (!latestEntry) {
        return 'empty'
    }
    switch (latestEntry.kind) {
    case 'message':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.status}:${latestEntry.content}:${latestEntry.timestamp}`
    case 'mode_change':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.mode}:${latestEntry.timestamp}`
    case 'context_compaction':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.status}:${latestEntry.content}:${latestEntry.timestamp}`
    case 'request_user_input':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.status}:${latestEntry.requestUserInput.status}:${JSON.stringify(latestEntry.requestUserInput.answers)}:${latestEntry.timestamp}`
    case 'tool_call':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.toolCall.status}:${latestEntry.toolCall.output || ''}:${latestEntry.timestamp}`
    case 'final_separator':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.label}:${latestEntry.timestamp}`
    case 'flow_run_request':
    case 'flow_launch':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.artifactId}:${latestEntry.timestamp}`
    }
}

const INHERITED_MODEL_SETTINGS: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }

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

    const resetComposerRef = useRef<() => void>(() => {})
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const inheritedModelSettings = useInheritedModelSettings(activeProjectPath)
    const {
        applyConversationSnapshot,
        commitConversationCache,
        conversationCache,
        conversationCacheRef,
        setConversationSummaryList,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        updateProjectSessionState,
    })
    const activeConversationRecord = activeConversationId
        ? conversationCache.conversationsById[activeConversationId] || null
        : null
    const isConversationHistoryLoading = Boolean(activeConversationId) && activeConversationRecord === null
    const {
        chatDraft,
        expandedThinkingEntries,
        expandedToolCalls,
        panelError,
        pendingConversationTurn,
        pendingDeleteConversationId,
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
    // Settings edits round-trip through the server before the conversation
    // snapshot updates; showing the requested values while the save is in
    // flight keeps the selects consistent (no stale model from the previous
    // provider shown under a freshly picked provider).
    const [chatSettingsDrafts, setChatSettingsDrafts] = useState<Record<string, ModelSettings>>({})
    const pendingChatSettings = activeConversationId ? chatSettingsDrafts[activeConversationId] ?? null : null
    const chatSettingsSaves = useRef(new Map<string, { values: ModelSettings | null }>())
    const {
        conversationBodyRef,
        homeSidebarRef,
        homeSidebarPrimaryHeight,
        isConversationPinnedToBottom,
        isHomeSidebarResizing,
        onHomeSidebarResizeKeyDown,
        onHomeSidebarResizePointerDown,
        scrollConversationToBottom,
        syncConversationPinnedState,
    } = useHomeSidebarLayout(isNarrowViewport, activeProjectPath, activeConversationId)
    const isConversationPinnedToBottomRef = useRef(isConversationPinnedToBottom)
    const projectsHomeViewModel = useMemo(() => buildProjectsHomeViewModel({
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        pendingConversationTurn,
        projectGitMetadata,
        uiDefaults,
    }), [
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        activeProjectScope,
        pendingConversationTurn,
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
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
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
    const isChatSubmissionDisabled = isChatInputDisabled || !isChatModelReady
    const conversationHistoryRevisionKey = useMemo(
        () => buildConversationHistoryRevisionKey(activeConversationHistory),
        [activeConversationHistory],
    )

    const activateConversationThread = useCallback((projectPath: string, conversationId: string, source = 'unknown') => {
        debugProjectChat('activate conversation thread', {
            source,
            projectPath,
            conversationId,
        })
        // Drafts belong to their project; only switching chats within the shown project clears one.
        if (projectPath === useStore.getState().activeProjectPath) resetComposerRef.current()
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
        isConversationPinnedToBottomRef.current = isConversationPinnedToBottom
    }, [isConversationPinnedToBottom])

    useEffect(() => {
        if (!pendingConversationTurn || !activeConversationRecord) {
            return
        }
        if (
            pendingConversationTurn.conversationId === activeConversationRecord.conversation_id
            && activeConversationRecord.revision > pendingConversationTurn.afterRevision
        ) {
            setPendingConversationTurn(null)
        }
    }, [activeConversationRecord, pendingConversationTurn, setPendingConversationTurn])

    useEffect(() => {
        if (!isConversationPinnedToBottomRef.current) {
            return
        }
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        node.scrollTop = node.scrollHeight
    }, [activeProjectPath, conversationBodyRef, conversationHistoryRevisionKey])

    const {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        resetComposer,
    } = useConversationComposer({
        activeProjectPath,
        chatDraft,
        isChatInputDisabled: isChatSubmissionDisabled,
        ensureConversationId,
        getCurrentConversationId: (projectPath) => (
            useStore.getState().projectSessionsByPath[projectPath]?.conversationId ?? null
        ),
        getCurrentConversationRevision: (conversationId) => (
            conversationCacheRef.current.conversationsById[conversationId]?.revision ?? 0
        ),
        applyConversationSnapshot,
        formatErrorMessage: extractApiErrorMessage,
        setChatDraft,
        setPanelError,
        setPendingConversationTurn,
    })

    useEffect(() => {
        resetComposerRef.current = resetComposer
    }, [resetComposer])

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
        let revision = String(conversationCacheRef.current.conversationsById[conversationId]?.revision ?? 0)
        try {
            // Coalesce edits while a save is in flight, then use its acknowledged revision.
            // A conflict stops the queue and retains the draft; never retry over another writer.
            while (true) {
                const saving = save.values
                const snapshot = await updateConversationSettingsValidated(conversationId, {
                    project_path: activeProjectPath,
                    expected_revision: revision,
                    model_settings: saving,
                })
                revision = String(snapshot.revision)
                applyConversationSnapshot(activeProjectPath, snapshot, 'chat-settings-response')
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
    }, [activeProjectPath, applyConversationSnapshot, ensureConversationId, setPanelError, conversationCacheRef, currentModelSettings])

    const onUseModelDefaults = () => {
        if (!activeProjectPath || !activeConversationId || pendingChatSettings) return
        void persistChatSettings(null)
    }

    const {
        onCreateConversationThread,
        onDeleteConversationThread,
        onSelectConversationThread,
    } = useProjectThreadActions({
        activeProjectPath,
        activeConversationId,
        conversationCacheRef,
        setConversationSummaryList,
        activateConversationThread,
        applyConversationSnapshot,
        resetComposer,
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
        applyConversationSnapshot,
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
            const snapshot = await submitConversationRequestUserInputValidated(
                activeConversationId,
                requestId,
                {
                    project_path: activeProjectPath,
                    answers,
                },
            )
            applyConversationSnapshot(activeProjectPath, snapshot, 'request-user-input-answer')
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
    }, [activeConversationId, activeProjectPath, applyConversationSnapshot])

    return {
        isNarrowViewport,
        historyProps: {
            activeConversationId,
            activeProjectPath,
            isConversationHistoryLoading,
            hasRenderableConversationHistory,
            activeConversationHistory,
            activeFlowRunRequestsById,
            activeFlowLaunchesById,
            activeProposedPlansById,
            latestFlowRunRequestId,
            latestFlowLaunchId,
            expandedToolCalls,
            expandedThinkingEntries,
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
        },
        sidebarProps: {
            isNarrowViewport,
            homeSidebarRef,
            homeSidebarPrimaryHeight,
            activeProjectPath,
            activeConversationId,
            conversationSummariesByProjectPath: conversationCache.summariesByProjectPath,
            conversationSummariesStatusByProjectPath: homeThreadSummariesStatusByProjectPath,
            pendingDeleteConversationId,
            isHomeSidebarResizing,
            onCreateConversationThread,
            onSelectConversationThread,
            onDeleteConversationThread,
            onHomeSidebarResizePointerDown,
            onHomeSidebarResizeKeyDown,
            formatConversationAgeShort,
            formatConversationTimestamp,
        },
        surfaceProps: {
            activeProjectLabel: activeProjectPath ? projectLabel(projectRegistry, activeProjectPath) : null,
            activeProjectPath,
            activeChatMode,
            // The picker edits what this conversation stores; effective settings would hide inheritance.
            modelSettings: editableModelSettings,
            // While nothing is stored, the conversation's effective settings are what it inherits.
            inheritedModelSettings: editableModelSettings === INHERITED_MODEL_SETTINGS ? currentModelSettings : inheritedModelSettings,
            defaultModel: isCodexProvider && !currentModelSettings.model ? activeProjectChatModel : undefined,
            onModelSettingsChange: (value: ModelSettings) => { void persistChatSettings(value) },
            chatModelAvailabilityMessage,
            hasRenderableConversationHistory,
            isConversationPinnedToBottom,
            isNarrowViewport,
            chatDraft,
            chatSendButtonLabel,
            isChatInputDisabled,
            isChatSendDisabled: isChatSubmissionDisabled,
            panelError: panelError || activeConversationRecord?.model_settings_view?.validation_errors?.join(' ') || null,
            conversationBodyRef,
            onSyncConversationPinnedState: syncConversationPinnedState,
            onScrollConversationToBottom: scrollConversationToBottom,
            onStopTurn: projectsHomeViewModel.hasActiveAssistantTurn && ['claude-code', 'claude_code'].includes(activeProjectChatProvider.trim().toLowerCase()) ? onStopTurn : undefined,
            onChatComposerSubmit,
            onChatComposerKeyDown,
            onChatDraftChange: setChatDraft,
            modelSettingsSource: activeConversationRecord?.model_settings_view?.source,
            onUseModelDefaults,
        },
    }
}
