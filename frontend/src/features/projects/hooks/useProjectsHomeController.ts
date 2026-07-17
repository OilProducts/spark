import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { getModelSuggestions } from '@/lib/llmSuggestions'
import { LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions'
import {
    fetchProjectChatModelsValidated,
    submitConversationRequestUserInputValidated,
    updateConversationSettingsValidated,
    type ProjectChatModelMetadataResponse,
    type ProjectChatModelsResponse,
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

const FALLBACK_REASONING_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']
const REASONING_EFFORT_LABELS: Record<string, string> = {
    low: 'Low',
    medium: 'Medium',
    high: 'High',
    xhigh: 'XHigh',
    max: 'Max',
    ultra: 'Ultra',
}

function dedupeOptions(options: Array<{ value: string; label: string }>) {
    const seen = new Set<string>()
    return options.filter((option) => {
        if (seen.has(option.value)) {
            return false
        }
        seen.add(option.value)
        return true
    })
}

function buildModelOptions(
    response: ProjectChatModelsResponse | undefined,
    selectedModel: string,
    provider: string,
) {
    const normalizedProvider = provider || 'codex'
    const metadataOptions = (response?.models || [])
        .filter((model) => (model.provider || 'codex') === normalizedProvider)
        .map((model) => ({
        value: model.id,
        label: model.display || model.id,
    }))
    if (normalizedProvider === 'codex') {
        if (!response) {
            return [{ value: '', label: 'Loading models...' }]
        }
        if (response.providers.codex.status === 'unavailable') {
            return [{ value: '', label: 'Models unavailable' }]
        }
        return metadataOptions.length > 0
            ? dedupeOptions(metadataOptions)
            : [{ value: '', label: 'No models available' }]
    }
    const fallbackOptions = getModelSuggestions(normalizedProvider).map((model) => ({
        value: model,
        label: model,
    }))
    const baseOptions = metadataOptions.length > 0 ? metadataOptions : fallbackOptions
    if (selectedModel && !baseOptions.some((option) => option.value === selectedModel)) {
        return [{ value: selectedModel, label: selectedModel }, ...baseOptions]
    }
    return baseOptions.length > 0 ? dedupeOptions(baseOptions) : [{ value: '', label: 'Default model' }]
}

function buildReasoningEffortOptions(
    models: ProjectChatModelMetadataResponse[],
    selectedModel: string,
    selectedEffort: string,
) {
    const selectedModelMetadata = models.find((model) => model.id === selectedModel)
    const metadataEfforts = selectedModelMetadata?.supported_reasoning_efforts || []
    const effortValues = metadataEfforts.length > 0 ? metadataEfforts : FALLBACK_REASONING_EFFORTS
    const defaultLabel = selectedModelMetadata?.default_reasoning_effort
        ? `Default (${REASONING_EFFORT_LABELS[selectedModelMetadata.default_reasoning_effort] || selectedModelMetadata.default_reasoning_effort})`
        : 'Default'
    return dedupeOptions([
        { value: '', label: defaultLabel },
        ...effortValues.map((effort) => ({
            value: effort,
            label: REASONING_EFFORT_LABELS[effort] || effort,
        })),
        selectedEffort ? {
            value: selectedEffort,
            label: REASONING_EFFORT_LABELS[selectedEffort] || selectedEffort,
        } : { value: '', label: defaultLabel },
    ])
}

export function useProjectsHomeController() {
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const homeThreadSummariesStatusByProjectPath = useStore((state) => state.homeThreadSummariesStatusByProjectPath)
    const clearHomeConversationSession = useStore((state) => state.clearHomeConversationSession)
    const setConversationId = useStore((state) => state.setConversationId)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const projectGitMetadata = useStore((state) => state.homeProjectGitMetadataByPath)
    const model = useStore((state) => state.model)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const resetComposerRef = useRef<() => void>(() => {})
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeConversationId = activeProjectScope?.conversationId ?? null
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
    const [chatModelsByProjectPath, setChatModelsByProjectPath] = useState<Record<string, ProjectChatModelsResponse>>({})
    // Settings edits round-trip through the server before the conversation
    // snapshot updates; showing the requested values while the save is in
    // flight keeps the selects consistent (no stale model from the previous
    // provider shown under a freshly picked provider).
    const [pendingChatSettings, setPendingChatSettings] = useState<{ provider: string; model: string; reasoningEffort: string } | null>(null)
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
    const activeProjectConversationSummariesStatus = activeProjectPath
        ? (homeThreadSummariesStatusByProjectPath[activeProjectPath] ?? 'idle')
        : 'idle'
    const projectsHomeViewModel = useMemo(() => buildProjectsHomeViewModel({
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        conversationCache,
        pendingConversationTurn,
        projectGitMetadata,
        uiDefaults,
    }), [
        activeConversationId,
        activeConversationRecord,
        activeProjectPath,
        activeProjectScope,
        conversationCache,
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
        activeProjectConversationSummaries,
        activeProjectLabel,
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
        latestFlowLaunchId,
        latestFlowRunRequestId,
    } = projectsHomeViewModel
    const activeProjectChatProvider = pendingChatSettings ? pendingChatSettings.provider : storedChatProvider
    const activeProjectChatModel = pendingChatSettings ? pendingChatSettings.model : storedChatModel
    const activeProjectChatReasoningEffort = pendingChatSettings
        ? pendingChatSettings.reasoningEffort
        : storedChatReasoningEffort
    const activeProjectChatModelsResponse = activeProjectPath
        ? chatModelsByProjectPath[activeProjectPath]
        : undefined
    const activeProjectChatModels = activeProjectChatModelsResponse?.models || []
    const chatModelOptions = useMemo(
        () => buildModelOptions(activeProjectChatModelsResponse, activeProjectChatModel, activeProjectChatProvider),
        [activeProjectChatModel, activeProjectChatModelsResponse, activeProjectChatProvider],
    )
    const isCodexProvider = (activeProjectChatProvider || 'codex') === 'codex'
    const codexModels = activeProjectChatModels.filter((model) => model.provider === 'codex')
    const isCodexDiscoveryUnavailable = isCodexProvider && (
        activeProjectChatModelsResponse?.providers.codex.status === 'unavailable'
        || (activeProjectChatModelsResponse?.providers.codex.status === 'available' && codexModels.length === 0)
    )
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
    const chatProviderOptions = useMemo(
        () => LLM_PROVIDER_OPTIONS.map((provider) => ({
            value: provider,
            label: provider === 'codex'
                ? 'Codex'
                : provider === 'claude-code'
                    ? 'Claude Code'
                : provider === 'openrouter'
                    ? 'OpenRouter'
                    : provider === 'litellm'
                        ? 'LiteLLM'
                        : provider[0].toUpperCase() + provider.slice(1),
        })),
        [],
    )
    const chatReasoningEffortOptions = useMemo(
        () => buildReasoningEffortOptions(
            activeProjectChatModels,
            activeProjectChatModel,
            activeProjectChatReasoningEffort,
        ),
        [activeProjectChatModel, activeProjectChatModels, activeProjectChatReasoningEffort],
    )
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
        resetComposerRef.current()
        setConversationId(conversationId)
        updateProjectSessionState(projectPath, { conversationId })
        void persistProjectState(projectPath, {
            active_conversation_id: conversationId,
            last_accessed_at: new Date().toISOString(),
        })
    }, [persistProjectState, setConversationId, updateProjectSessionState])

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
        resetComposerRef.current()
    }, [activeProjectPath])

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

    useEffect(() => {
        if (!activeProjectPath || activeProjectPath in chatModelsByProjectPath) {
            return
        }
        let isCancelled = false
        const loadChatModels = async () => {
            try {
                const payload = await fetchProjectChatModelsValidated(activeProjectPath)
                if (!isCancelled) {
                    setChatModelsByProjectPath((current) => ({
                        ...current,
                        [activeProjectPath]: payload,
                    }))
                }
            } catch (error) {
                if (!isCancelled) {
                    setChatModelsByProjectPath((current) => ({
                        ...current,
                        [activeProjectPath]: {
                            models: [],
                            providers: {
                                codex: {
                                    status: 'unavailable',
                                    error: extractApiErrorMessage(error, 'Unable to discover Codex models.'),
                                },
                            },
                        },
                    }))
                }
            }
        }
        void loadChatModels()
        return () => {
            isCancelled = true
        }
    }, [activeProjectPath, chatModelsByProjectPath])

    const {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        resetComposer,
    } = useConversationComposer({
        activeProjectPath,
        chatDraft,
        isChatInputDisabled: isChatSubmissionDisabled,
        model: activeProjectChatModel,
        provider: activeProjectChatProvider,
        reasoningEffort: activeProjectChatReasoningEffort,
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

    const persistChatSettings = useCallback(async (values: { provider: string; model: string; reasoningEffort: string }) => {
        if (!activeProjectPath) {
            return
        }
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }
        setPanelError(null)
        setPendingChatSettings(values)
        try {
            const snapshot = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                provider: values.provider.trim() || 'codex',
                model: values.model.trim() || null,
                reasoning_effort: values.reasoningEffort.trim() || '',
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'chat-settings-response', {
                forceWorkspaceSync: true,
            })
        } catch (error) {
            const message = extractApiErrorMessage(error, 'Unable to update the project chat settings.')
            setPanelError(message)
        } finally {
            // Only clear our own pending values; a newer edit may already be
            // in flight with its own optimistic state.
            setPendingChatSettings((current) => (current === values ? null : current))
        }
    }, [activeProjectPath, applyConversationSnapshot, ensureConversationId, setPanelError])

    const onChatModelChange = useCallback((value: string) => {
        void persistChatSettings({
            provider: activeProjectChatProvider,
            model: value,
            reasoningEffort: activeProjectChatReasoningEffort,
        })
    }, [activeProjectChatProvider, activeProjectChatReasoningEffort, persistChatSettings])

    const onChatProviderChange = useCallback((value: string) => {
        void persistChatSettings({
            provider: value || 'codex',
            model: '',
            reasoningEffort: activeProjectChatReasoningEffort,
        })
    }, [activeProjectChatReasoningEffort, persistChatSettings])

    const onChatReasoningEffortChange = useCallback((value: string) => {
        void persistChatSettings({
            provider: activeProjectChatProvider,
            model: activeProjectChatModel,
            reasoningEffort: value,
        })
    }, [activeProjectChatProvider, activeProjectChatModel, persistChatSettings])

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
        resetComposer,
        setConversationId,
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
        setSelectedRunId(request.run_id)
        setViewMode('runs')
    }, [setSelectedRunId, setViewMode])

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
            activeProjectLabel,
            activeProjectConversationSummaries,
            activeProjectConversationSummariesStatus,
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
            activeProjectLabel,
            activeProjectPath,
            activeChatMode,
            activeChatProvider: activeProjectChatProvider,
            activeChatModel: activeProjectChatModel,
            activeChatReasoningEffort: activeProjectChatReasoningEffort,
            chatModelOptions,
            chatProviderOptions,
            chatReasoningEffortOptions,
            chatModelAvailabilityMessage,
            hasRenderableConversationHistory,
            isConversationPinnedToBottom,
            isNarrowViewport,
            chatDraft,
            chatSendButtonLabel,
            isChatInputDisabled,
            isChatModelSelectDisabled: isCodexDiscoveryUnavailable,
            isChatSendDisabled: isChatSubmissionDisabled,
            panelError,
            conversationBodyRef,
            onSyncConversationPinnedState: syncConversationPinnedState,
            onScrollConversationToBottom: scrollConversationToBottom,
            onChatComposerSubmit,
            onChatComposerKeyDown,
            onChatDraftChange: setChatDraft,
            onChatModelChange,
            onChatProviderChange,
            onChatReasoningEffortChange,
        },
    }
}
