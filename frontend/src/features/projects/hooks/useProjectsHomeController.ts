import { useCallback, useEffect, useMemo, useRef } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useHomeSidebarLayout } from './useHomeSidebarLayout'
import { useConversationStream } from './useConversationStream'
import { useConversationComposer } from './useConversationComposer'
import { useConversationReviews } from './useConversationReviews'
import { useProjectConversationCache } from './useProjectConversationCache'
import { useProjectGitMetadata } from './useProjectGitMetadata'
import { useProjectsHomeInteractionState } from './useProjectsHomeInteractionState'
import { usePersistProjectState } from './usePersistProjectState'
import { useProjectThreadActions } from './projectThreadActions'
import { debugProjectChat } from '../model/projectChatDebug'
import { buildProjectsHomeViewModel } from '../model/projectsHomeViewModel'
import type { ConversationTimelineEntry } from '../model/types'
import {
    buildOrderedProjects,
    buildProjectConversationId,
    extractApiErrorMessage,
    formatConversationAgeShort,
    formatConversationTimestamp,
    removeProjectFromCache,
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
    const projectRegistry = useStore((state) => state.projectRegistry)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const projects = Object.values(projectRegistry)
    const recentProjectPaths = useStore((state) => state.recentProjectPaths)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const model = useStore((state) => state.model)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const resetComposerRef = useRef<() => void>(() => {})
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)

    const isNarrowViewport = useNarrowViewport()
    const { projectGitMetadata, setProjectGitMetadata, ensureProjectGitRepository } = useProjectGitMetadata({
        projectPaths: projects.map((project) => project.directoryPath),
        setProjectRegistrationError: () => {},
    })
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const {
        applyConversationSnapshot,
        applyConversationStreamEvent,
        commitConversationCache,
        conversationCache,
        conversationCacheRef,
        loadProjectConversationSummaries,
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
    const latestConversationSpecEditProposalId = activeConversationSnapshot?.spec_edit_proposals.at(-1)?.id || null
    const {
        chatDraft,
        expandedProposalChanges,
        expandedThinkingEntries,
        expandedToolCalls,
        optimisticSend,
        panelError,
        pendingDeleteConversationId,
        pendingDeleteProjectPath,
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
    } = useHomeSidebarLayout(isNarrowViewport, activeProjectPath)
    const isConversationPinnedToBottomRef = useRef(isConversationPinnedToBottom)

    const orderedProjects = useMemo(
        () => buildOrderedProjects(projects, projectRegistry, recentProjectPaths),
        [projectRegistry, projects, recentProjectPaths],
    )
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

    useEffect(() => {
        const registeredPaths = new Set(Object.keys(projectRegistry))
        commitConversationCache((current) => {
            const removableProjectPaths = Object.keys(current.summariesByProjectPath).filter(
                (projectPath) => !registeredPaths.has(projectPath),
            )
            if (removableProjectPaths.length === 0) {
                return current
            }
            return removableProjectPaths.reduce(
                (next, projectPath) => removeProjectFromCache(next, projectPath),
                current,
            )
        })
        setProjectGitMetadata((current) => {
            const next = Object.fromEntries(
                Object.entries(current).filter(([projectPath]) => registeredPaths.has(projectPath)),
            )
            return Object.keys(next).length === Object.keys(current).length ? current : next
        })
    }, [commitConversationCache, projectRegistry, setProjectGitMetadata])

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

    useConversationStream({
        activeConversationId,
        activeProjectPath,
        appendLocalProjectEvent,
        applyConversationSnapshot,
        applyConversationStreamEvent,
        formatErrorMessage: extractApiErrorMessage,
        setPanelError,
    })

    useEffect(() => {
        if (!activeProjectPath) {
            return
        }

        let isCancelled = false
        const loadThreadSummaries = async () => {
            const summaries = await loadProjectConversationSummaries(activeProjectPath)
            if (isCancelled) {
                return
            }
            if (activeConversationId) {
                return
            }
            const latestConversation = summaries[0] || null
            if (latestConversation) {
                activateConversationThread(activeProjectPath, latestConversation.conversation_id, 'load-latest-thread')
            }
        }

        void loadThreadSummaries()
        return () => {
            isCancelled = true
        }
    }, [activeConversationId, activeProjectPath, activateConversationThread, loadProjectConversationSummaries])

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
