import { useCallback, useEffect, useMemo, useRef } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { PendingInterviewGateGroup } from '@/features/runs/model/shared'
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

const PROJECT_CHAT_PENDING_QUESTION_PREVIEW_GROUPS: PendingInterviewGateGroup[] = [
    {
        key: 'project-chat-request-user-input-preview',
        heading: 'Preview request_user_input',
        gates: [
            {
                eventId: 'preview-multiple-choice',
                sequence: 1,
                receivedAt: '2026-04-17T12:00:00Z',
                nodeId: null,
                stageIndex: null,
                sourceScope: 'root',
                sourceParentNodeId: null,
                sourceFlowName: null,
                prompt: 'Which planning path should I take for this change?',
                questionId: 'preview-multiple-choice',
                questionType: 'MULTIPLE_CHOICE',
                options: [
                    {
                        label: 'Inline card only',
                        value: 'inline-card',
                        key: 'A',
                        description: 'Keep the request inside the timeline.',
                    },
                    {
                        label: 'Composer takeover',
                        value: 'composer-takeover',
                        key: 'B',
                        description: 'Turn the bottom input into the answer surface.',
                    },
                    {
                        label: 'Hybrid',
                        value: 'hybrid',
                        key: 'C',
                        description: 'Keep the timeline card and mirror controls in the composer.',
                    },
                ],
            },
            {
                eventId: 'preview-freeform',
                sequence: 2,
                receivedAt: '2026-04-17T12:00:10Z',
                nodeId: null,
                stageIndex: null,
                sourceScope: 'root',
                sourceParentNodeId: null,
                sourceFlowName: null,
                prompt: 'What context or constraints should the final plan preserve?',
                questionId: 'preview-freeform',
                questionType: 'FREEFORM',
                options: [],
            },
        ],
    },
]

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
    case 'tool_call':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.toolCall.status}:${latestEntry.toolCall.output || ''}:${latestEntry.timestamp}`
    case 'final_separator':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.label}:${latestEntry.timestamp}`
    case 'flow_run_request':
    case 'flow_launch':
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.artifactId}:${latestEntry.timestamp}`
    }
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
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
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
    const {
        activeConversationHistory,
        activeChatMode,
        activeFlowLaunchesById,
        activeFlowRunRequestsById,
        activeProjectConversationSummaries,
        activeProjectEventLog,
        activeProjectLabel,
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
        latestFlowLaunchId,
        latestFlowRunRequestId,
    } = useMemo(() => buildProjectsHomeViewModel({
        activeConversationId,
        activeConversationSnapshot,
        activeProjectPath,
        activeProjectScope,
        conversationCache,
        optimisticSend,
        projectGitMetadata,
    }), [
        activeConversationId,
        activeConversationSnapshot,
        activeProjectPath,
        activeProjectScope,
        conversationCache,
        optimisticSend,
        projectGitMetadata,
    ])
    const pendingQuestionPreviewGroups = useMemo(
        () => (activeConversationId && activeChatMode === 'plan' ? PROJECT_CHAT_PENDING_QUESTION_PREVIEW_GROUPS : []),
        [activeConversationId, activeChatMode],
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

    const {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        resetComposer,
    } = useConversationComposer({
        activeProjectPath,
        chatDraft,
        isChatInputDisabled,
        model,
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
        pendingFlowRunRequestId,
    } = useConversationReviews({
        activeConversationId,
        activeProjectPath,
        appendLocalProjectEvent,
        applyConversationSnapshot,
        formatErrorMessage: extractApiErrorMessage,
        model,
        setPanelError,
    })

    const onOpenFlowRun = (request: { run_id?: string | null; flow_name: string }) => {
        if (!request.run_id) {
            return
        }
        setSelectedRunId(request.run_id)
        setExecutionFlow(request.flow_name || null)
        setViewMode('runs')
    }

    return {
        isNarrowViewport,
        historyProps: {
            activeConversationId,
            isConversationHistoryLoading,
            hasRenderableConversationHistory,
            activeConversationHistory,
            pendingQuestionPreviewGroups,
            activeFlowRunRequestsById,
            activeFlowLaunchesById,
            latestFlowRunRequestId,
            latestFlowLaunchId,
            expandedToolCalls,
            expandedThinkingEntries,
            pendingFlowRunRequestId,
            formatConversationTimestamp,
            onToggleToolCallExpanded: toggleToolCallExpanded,
            onToggleThinkingEntryExpanded: toggleThinkingEntryExpanded,
            onReviewFlowRunRequest,
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
        },
    }
}
