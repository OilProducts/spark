import { useCallback, useEffect, useMemo, useRef } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useHomeSidebarLayout } from './useHomeSidebarLayout'
import { useConversationComposer } from './useConversationComposer'
import { useConversationReviews } from './useConversationReviews'
import { useProjectConversationCache } from './useProjectConversationCache'
import { useProjectsHomeInteractionState } from './useProjectsHomeInteractionState'
import { usePersistProjectState } from './usePersistProjectState'
import { useProjectThreadActions } from './projectThreadActions'
import { debugProjectChat } from '../model/projectChatDebug'
import { buildProjectsHomeViewModel } from '../model/projectsHomeViewModel'
import type { ProjectGitMetadata } from '../model/presentation'
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
    if (latestEntry.kind === 'message') {
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.status}:${latestEntry.content}:${latestEntry.timestamp}`
    }
    if (latestEntry.kind === 'tool_call') {
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.toolCall.status}:${latestEntry.toolCall.output || ''}:${latestEntry.timestamp}`
    }
    if (latestEntry.kind === 'final_separator') {
        return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.label}:${latestEntry.timestamp}`
    }
    return `${history.length}:${latestEntry.kind}:${latestEntry.id}:${latestEntry.artifactId}:${latestEntry.timestamp}`
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
    const setHomeProjectGitMetadata = useStore((state) => state.setHomeProjectGitMetadata)
    const model = useStore((state) => state.model)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const resetComposerRef = useRef<() => void>(() => {})
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const setProjectGitMetadata = useCallback((next: Record<string, ProjectGitMetadata> | ((current: Record<string, ProjectGitMetadata>) => Record<string, ProjectGitMetadata>)) => {
        const current = useStore.getState().homeProjectGitMetadataByPath
        const resolved = typeof next === 'function' ? next(current) : next
        Object.entries(resolved).forEach(([projectPath, metadata]) => {
            setHomeProjectGitMetadata(projectPath, metadata)
        })
    }, [setHomeProjectGitMetadata])
    const {
        applyConversationSnapshot,
        commitConversationCache,
        conversationCache,
        conversationCacheRef,
        setConversationSummaryList,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        setProjectGitMetadata,
        updateProjectSessionState,
    })
    const activeConversationSnapshot = activeConversationId
        ? conversationCache.snapshotsByConversationId[activeConversationId] || null
        : null
    const isConversationHistoryLoading = Boolean(activeConversationId) && activeConversationSnapshot === null
    const latestConversationSpecEditProposalId = activeConversationSnapshot?.spec_edit_proposals.at(-1)?.id || null
    const {
        chatDraft,
        expandedProposalChanges,
        expandedThinkingEntries,
        expandedToolCalls,
        optimisticSend,
        panelError,
        pendingDeleteConversationId,
        setChatDraft,
        setOptimisticSend,
        setPanelError,
        setPendingDeleteConversationId,
        toggleProposalChangeExpanded,
        toggleThinkingEntryExpanded,
        toggleToolCallExpanded,
    } = useProjectsHomeInteractionState({
        activeConversationId,
        activeProjectPath,
        latestSpecEditProposalId: latestConversationSpecEditProposalId,
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
        activeExecutionCardsById,
        activeFlowLaunchesById,
        activeFlowRunRequestsById,
        activeProjectConversationSummaries,
        activeProjectEventLog,
        activeProjectGitMetadata,
        activeProjectLabel,
        activeSpecEditProposalsById,
        chatSendButtonLabel,
        hasRenderableConversationHistory,
        isChatInputDisabled,
        latestExecutionCardId,
        latestFlowLaunchId,
        latestFlowRunRequestId,
        latestSpecEditProposalId,
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
        updateProjectSessionState(projectPath, {
            conversationId,
            specId: null,
            specStatus: 'draft',
            specProvenance: null,
            planId: null,
            planStatus: 'draft',
            planProvenance: null,
        })
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
        onApproveSpecEditProposal,
        onRejectSpecEditProposal,
        onReviewExecutionCard,
        onReviewFlowRunRequest,
        pendingExecutionCardId,
        pendingFlowRunRequestId,
        pendingSpecProposalId,
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
            activeSpecEditProposalsById,
            activeFlowRunRequestsById,
            activeFlowLaunchesById,
            activeExecutionCardsById,
            latestSpecEditProposalId,
            latestFlowRunRequestId,
            latestFlowLaunchId,
            latestExecutionCardId,
            activeProjectGitMetadata,
            expandedToolCalls,
            expandedThinkingEntries,
            expandedProposalChanges,
            pendingSpecProposalId,
            pendingFlowRunRequestId,
            pendingExecutionCardId,
            formatConversationTimestamp,
            onToggleToolCallExpanded: toggleToolCallExpanded,
            onToggleThinkingEntryExpanded: toggleThinkingEntryExpanded,
            onToggleProposalChangeExpanded: toggleProposalChangeExpanded,
            onApproveSpecEditProposal,
            onRejectSpecEditProposal,
            onReviewFlowRunRequest,
            onOpenFlowRun,
            onReviewExecutionCard,
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
