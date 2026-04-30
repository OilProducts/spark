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
import {
    stabilizeConversationTimelineEntries,
    type ConversationTimelineStabilizationScope,
} from '../model/conversationTimeline'
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

const FALLBACK_REASONING_EFFORTS = ['low', 'medium', 'high', 'xhigh']
const REASONING_EFFORT_LABELS: Record<string, string> = {
    low: 'Low',
    medium: 'Medium',
    high: 'High',
    xhigh: 'XHigh',
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
    models: ProjectChatModelMetadataResponse[],
    selectedModel: string,
    provider: string,
) {
    const normalizedProvider = provider || 'codex'
    const metadataOptions = models
        .filter((model) => (model.provider || 'codex') === normalizedProvider)
        .map((model) => ({
        value: model.id,
        label: model.display || model.id,
    }))
    const fallbackOptions = getModelSuggestions(provider).map((model) => ({
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
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const projectGitMetadata = useStore((state) => state.homeProjectGitMetadataByPath)
    const model = useStore((state) => state.model)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const resetComposerRef = useRef<() => void>(() => {})
    const previousConversationHistoryScopeRef = useRef<ConversationTimelineStabilizationScope | null>(null)
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
    const activeConversationSnapshot = activeConversationId
        ? conversationCache.snapshotsByConversationId[activeConversationId] || null
        : null
    const isConversationHistoryLoading = Boolean(activeConversationId) && activeConversationSnapshot === null
    const {
        chatDraft,
        expandedThinkingEntries,
        expandedToolCalls,
        optimisticSend,
        panelError,
        pendingDeleteConversationId,
        setChatDraft,
        setOptimisticSend,
        setPanelError,
        setPendingDeleteConversationId,
        toggleThinkingEntryExpanded,
        toggleToolCallExpanded,
    } = useProjectsHomeInteractionState({
        activeConversationId,
        activeProjectPath,
    })
    const [requestUserInputActionError, setRequestUserInputActionError] = useState<string | null>(null)
    const [submittingRequestUserInputIds, setSubmittingRequestUserInputIds] = useState<Record<string, boolean>>({})
    const [chatModelMetadataByProjectPath, setChatModelMetadataByProjectPath] = useState<Record<string, ProjectChatModelMetadataResponse[]>>({})
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
        activeConversationSnapshot,
        activeProjectPath,
        activeProjectScope,
        conversationCache,
        optimisticSend,
        projectGitMetadata,
        uiDefaults,
    }), [
        activeConversationId,
        activeConversationSnapshot,
        activeProjectPath,
        activeProjectScope,
        conversationCache,
        optimisticSend,
        projectGitMetadata,
        uiDefaults,
    ])
    const activeConversationHistory = useMemo(
        () => stabilizeConversationTimelineEntries(
            activeConversationId,
            projectsHomeViewModel.activeConversationHistory,
            previousConversationHistoryScopeRef.current,
        ),
        [activeConversationId, projectsHomeViewModel.activeConversationHistory],
    )
    useEffect(() => {
        previousConversationHistoryScopeRef.current = {
            conversationId: activeConversationId,
            entries: activeConversationHistory,
        }
    }, [activeConversationId, activeConversationHistory])
    const {
        activeChatMode,
        activeProjectChatProvider,
        activeProjectChatModel,
        activeProjectChatReasoningEffort,
        activeFlowLaunchesById,
        activeFlowRunRequestsById,
        activeProposedPlansById,
        activeProjectConversationSummaries,
        activeProjectEventLog,
        activeProjectLabel,
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
        latestFlowLaunchId,
        latestFlowRunRequestId,
    } = projectsHomeViewModel
    const activeProjectChatModels = activeProjectPath
        ? chatModelMetadataByProjectPath[activeProjectPath] || []
        : []
    const chatModelOptions = useMemo(
        () => buildModelOptions(activeProjectChatModels, activeProjectChatModel, activeProjectChatProvider),
        [activeProjectChatModel, activeProjectChatModels, activeProjectChatProvider],
    )
    const chatProviderOptions = useMemo(
        () => LLM_PROVIDER_OPTIONS.map((provider) => ({
            value: provider,
            label: provider === 'codex'
                ? 'Codex'
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

    const appendLocalProjectEvent = useCallback((message: string) => {
        appendProjectEventEntry({
            message,
            timestamp: new Date().toISOString(),
        })
    }, [appendProjectEventEntry])

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
        if (!activeProjectPath || activeProjectPath in chatModelMetadataByProjectPath) {
            return
        }
        let isCancelled = false
        const loadChatModels = async () => {
            try {
                const payload = await fetchProjectChatModelsValidated(activeProjectPath)
                if (!isCancelled) {
                    setChatModelMetadataByProjectPath((current) => ({
                        ...current,
                        [activeProjectPath]: payload.models,
                    }))
                }
            } catch {
                if (!isCancelled) {
                    setChatModelMetadataByProjectPath((current) => ({
                        ...current,
                        [activeProjectPath]: [],
                    }))
                }
            }
        }
        void loadChatModels()
        return () => {
            isCancelled = true
        }
    }, [activeProjectPath, chatModelMetadataByProjectPath])

    const {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        resetComposer,
    } = useConversationComposer({
        activeProjectPath,
        chatDraft,
        isChatInputDisabled,
        model: activeProjectChatModel,
        provider: activeProjectChatProvider,
        reasoningEffort: activeProjectChatReasoningEffort,
        ensureConversationId,
        getCurrentConversationId: (projectPath) => (
            useStore.getState().projectSessionsByPath[projectPath]?.conversationId ?? null
        ),
        applyConversationSnapshot,
        appendLocalProjectEvent,
        formatErrorMessage: extractApiErrorMessage,
        setChatDraft,
        setPanelError,
        setOptimisticSend,
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
            appendLocalProjectEvent(`Project chat settings update failed: ${message}`)
        }
    }, [activeProjectPath, appendLocalProjectEvent, applyConversationSnapshot, ensureConversationId, setPanelError])

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
        applyConversationSnapshot,
        activateConversationThread,
        resetComposer,
        setConversationId,
        updateProjectSessionState,
        clearHomeConversationSession,
        setPanelError,
        setPendingDeleteConversationId,
        appendLocalProjectEvent,
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
        appendLocalProjectEvent,
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
        setExecutionFlow(request.flow_name || null)
        setViewMode('runs')
    }, [setExecutionFlow, setSelectedRunId, setViewMode])

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
            appendLocalProjectEvent(`Project chat request_user_input failed: ${message}`)
        } finally {
            setSubmittingRequestUserInputIds((current) => {
                const next = { ...current }
                delete next[requestId]
                return next
            })
        }
    }, [activeConversationId, activeProjectPath, appendLocalProjectEvent, applyConversationSnapshot])

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
            activeProjectEventLog,
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
            hasRenderableConversationHistory,
            isConversationPinnedToBottom,
            isNarrowViewport,
            chatDraft,
            chatSendButtonLabel,
            isChatInputDisabled,
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
