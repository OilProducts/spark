import { type ChangeEvent, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import {
    ApiHttpError,
    type ConversationSegmentUpsertEventResponse,
    type ConversationSnapshotResponse,
    type ConversationSummaryResponse,
    type ConversationTurnUpsertEventResponse,
    deleteConversationValidated,
    deleteProjectValidated,
    fetchProjectConversationListValidated,
    fetchProjectMetadataValidated,
    fetchProjectRegistryValidated,
    pickProjectDirectoryValidated,
    registerProjectValidated,
    updateProjectStateValidated,
} from '@/lib/workspaceClient'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { ProjectGitMetadata } from '@/components/projects/presentation'
import { buildConversationTimelineEntries } from '@/components/projects/conversationTimeline'
import {
    type OptimisticSendState,
} from '@/components/projects/conversationState'
import { useHomeSidebarLayout } from '@/components/projects/hooks/useHomeSidebarLayout'
import { useConversationStream } from '@/components/projects/hooks/useConversationStream'
import { useConversationComposer } from '@/components/projects/hooks/useConversationComposer'
import { useConversationReviews } from '@/components/projects/hooks/useConversationReviews'
import {
    applyConversationSnapshotToCache,
    applyConversationStreamEventToCache,
    asProjectGitMetadataField,
    buildOrderedProjects,
    buildProjectConversationId,
    derivePlanStatusFromExecutionCard,
    deriveProjectPathFromDirectorySelection,
    EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
    EMPTY_PROJECT_GIT_METADATA,
    extractApiErrorMessage,
    formatConversationAgeShort,
    formatConversationTimestamp,
    formatProjectListLabel,
    removeConversationFromCache,
    removeProjectFromCache,
    resolveProjectPathValidation,
    setProjectConversationSummaryList,
    toHydratedProjectRecord,
    type ConversationStreamEvent,
    type ProjectConversationCacheState,
} from '@/components/projects/projectsHomeState'

const isProjectChatDebugEnabled = () => {
    if (typeof window === 'undefined') {
        return false
    }
    try {
        const params = new URLSearchParams(window.location.search)
        if (params.get('debugProjectChat') === '1') {
            return true
        }
        return window.localStorage.getItem('spark.debug.project_chat') === '1'
    } catch {
        return false
    }
}

const summarizeConversationTurnsForDebug = (turns: ConversationSnapshotResponse['turns']) => (
    turns.map((turn, index) => ({
        index,
        id: turn.id,
        role: turn.role,
        kind: turn.kind,
        status: turn.status,
        artifactId: turn.artifact_id ?? null,
        content: turn.content.slice(0, 120),
    }))
)

const debugProjectChat = (message: string, details?: Record<string, unknown>) => {
    if (!isProjectChatDebugEnabled()) {
        return
    }
    if (details) {
        console.debug(`[project-chat] ${message}`, details)
        return
    }
    console.debug(`[project-chat] ${message}`)
}

export function useProjectsHomeController() {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const projects = Object.values(projectRegistry)
    const recentProjectPaths = useStore((state) => state.recentProjectPaths)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const registerProject = useStore((state) => state.registerProject)
    const removeProject = useStore((state) => state.removeProject)
    const setProjectRegistrationError = useStore((state) => state.setProjectRegistrationError)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const model = useStore((state) => state.model)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [conversationCache, setConversationCache] = useState<ProjectConversationCacheState>(
        EMPTY_PROJECT_CONVERSATION_CACHE_STATE,
    )
    const [chatDraft, setChatDraft] = useState('')
    const [panelError, setPanelError] = useState<string | null>(null)
    const [optimisticSend, setOptimisticSend] = useState<OptimisticSendState | null>(null)
    const [pendingDeleteConversationId, setPendingDeleteConversationId] = useState<string | null>(null)
    const [pendingDeleteProjectPath, setPendingDeleteProjectPath] = useState<string | null>(null)
    const [expandedProposalChanges, setExpandedProposalChanges] = useState<Record<string, boolean>>({})
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinkingEntries, setExpandedThinkingEntries] = useState<Record<string, boolean>>({})

    const projectDirectoryPickerInputRef = useRef<HTMLInputElement | null>(null)
    const conversationCacheRef = useRef(conversationCache)
    conversationCacheRef.current = conversationCache

    const commitConversationCache = (
        next:
            | ProjectConversationCacheState
            | ((current: ProjectConversationCacheState) => ProjectConversationCacheState),
    ) => {
        const resolved = typeof next === 'function'
            ? next(conversationCacheRef.current)
            : next
        conversationCacheRef.current = resolved
        setConversationCache(resolved)
    }

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectSessionsByPath[activeProjectPath] : null
    const activeProjectLabel = activeProjectPath ? formatProjectListLabel(activeProjectPath) : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const activeConversationSnapshot = activeConversationId
        ? conversationCache.snapshotsByConversationId[activeConversationId] || null
        : null
    const activeProjectConversationSummaries = activeProjectPath
        ? conversationCache.summariesByProjectPath[activeProjectPath] || []
        : []
    const activeProjectEventLog = activeProjectScope?.projectEventLog || []
    const activeConversationHistory = useMemo(
        () => buildConversationTimelineEntries(
            activeConversationSnapshot,
            optimisticSend && optimisticSend.conversationId === activeConversationId ? optimisticSend : null,
        ),
        [activeConversationId, activeConversationSnapshot, optimisticSend],
    )
    const activeSpecEditProposals = activeConversationSnapshot?.spec_edit_proposals || []
    const activeFlowRunRequests = activeConversationSnapshot?.flow_run_requests || []
    const activeFlowLaunches = activeConversationSnapshot?.flow_launches || []
    const activeExecutionCards = activeConversationSnapshot?.execution_cards || []
    const latestSpecEditProposalId = activeSpecEditProposals.length > 0
        ? activeSpecEditProposals[activeSpecEditProposals.length - 1]?.id || null
        : null
    const latestFlowRunRequestId = activeFlowRunRequests.length > 0
        ? activeFlowRunRequests[activeFlowRunRequests.length - 1]?.id || null
        : null
    const latestFlowLaunchId = activeFlowLaunches.length > 0
        ? activeFlowLaunches[activeFlowLaunches.length - 1]?.id || null
        : null
    const latestExecutionCardId = activeExecutionCards.length > 0
        ? activeExecutionCards[activeExecutionCards.length - 1]?.id || null
        : null
    const activeSpecEditProposalsById = new Map(activeSpecEditProposals.map((proposal) => [proposal.id, proposal]))
    const activeFlowRunRequestsById = new Map(activeFlowRunRequests.map((request) => [request.id, request]))
    const activeFlowLaunchesById = new Map(activeFlowLaunches.map((launch) => [launch.id, launch]))
    const activeExecutionCardsById = new Map(activeExecutionCards.map((executionCard) => [executionCard.id, executionCard]))
    const hasRenderableConversationHistory = activeConversationHistory.some((entry) => (
        entry.kind === 'spec_edit_proposal'
        || entry.kind === 'flow_run_request'
        || entry.kind === 'flow_launch'
        || entry.kind === 'execution_card'
        || entry.kind === 'tool_call'
        || entry.role === 'user'
        || entry.role === 'assistant'
    ))
    const hasActiveAssistantTurn = (activeConversationSnapshot?.turns || []).some((turn) => (
        turn.role === 'assistant' && (turn.status === 'pending' || turn.status === 'streaming')
    ))
    const isChatInputDisabled = hasActiveAssistantTurn
    const chatSendButtonLabel = hasActiveAssistantTurn ? 'Thinking...' : 'Send'
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

    const orderedProjects = useMemo(
        () => buildOrderedProjects(projects, projectRegistry, recentProjectPaths),
        [projectRegistry, projects, recentProjectPaths],
    )

    const appendLocalProjectEvent = (message: string) => {
        appendProjectEventEntry({
            message,
            timestamp: new Date().toISOString(),
        })
    }

    const setConversationSummaryList = (
        projectPath: string,
        summaries: ConversationSummaryResponse[],
    ) => {
        commitConversationCache((current) => setProjectConversationSummaryList(current, projectPath, summaries))
    }

    const persistProjectState = async (
        projectPath: string,
        patch: {
            last_accessed_at?: string | null
            active_conversation_id?: string | null
            is_favorite?: boolean | null
        },
    ) => {
        try {
            const project = await updateProjectStateValidated({
                project_path: projectPath,
                ...patch,
            })
            upsertProjectRegistryEntry(toHydratedProjectRecord(project))
        } catch {
            // Keep the UI responsive if the background state sync fails.
        }
    }

    const activateConversationThread = (projectPath: string, conversationId: string, source = 'unknown') => {
        debugProjectChat('activate conversation thread', {
            source,
            projectPath,
            conversationId,
        })
        resetComposer()
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
    }

    const loadProjectConversationSummaries = async (projectPath: string) => {
        try {
            const summaries = await fetchProjectConversationListValidated(projectPath)
            setConversationSummaryList(projectPath, summaries)
            return summaries
        } catch {
            return conversationCacheRef.current.summariesByProjectPath[projectPath] || []
        }
    }

    const applyConversationSnapshot = (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source = 'unknown',
        options?: {
            forceWorkspaceSync?: boolean
        },
    ) => {
        const latestProjectScope = useStore.getState().projectSessionsByPath[projectPath]
        const shouldSyncActiveWorkspace = options?.forceWorkspaceSync === true
            || latestProjectScope?.conversationId === snapshot.conversation_id
        const { applied, cache, latestApprovedProposal, latestExecutionCard } = applyConversationSnapshotToCache(
            conversationCacheRef.current,
            projectPath,
            snapshot,
        )
        if (!applied) {
            debugProjectChat('skip stale conversation snapshot', {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                snapshotUpdatedAt: snapshot.updated_at,
            })
            return
        }
        debugProjectChat('apply conversation snapshot', {
            source,
            projectPath,
            snapshotProjectPath: snapshot.project_path,
            conversationId: snapshot.conversation_id,
            shouldSyncActiveWorkspace,
            turnCount: snapshot.turns.length,
            turns: summarizeConversationTurnsForDebug(snapshot.turns),
        })
        commitConversationCache(cache)

        if (shouldSyncActiveWorkspace) {
            updateProjectSessionState(projectPath, {
                conversationId: snapshot.conversation_id,
                projectEventLog: snapshot.event_log.map((entry) => ({
                    message: entry.message,
                    timestamp: entry.timestamp,
                })),
                specId: latestApprovedProposal?.canonical_spec_edit_id ?? null,
                specStatus: latestApprovedProposal ? 'approved' : 'draft',
                specProvenance: latestApprovedProposal
                    ? {
                        source: 'spec-edit-proposal',
                        referenceId: latestApprovedProposal.id,
                        capturedAt: latestApprovedProposal.approved_at || latestApprovedProposal.created_at,
                        runId: null,
                        gitBranch: latestApprovedProposal.git_branch ?? null,
                        gitCommit: latestApprovedProposal.git_commit ?? null,
                    }
                    : null,
                planId: latestExecutionCard?.id ?? null,
                planStatus: derivePlanStatusFromExecutionCard(latestExecutionCard),
                planProvenance: latestExecutionCard
                    ? {
                        source: 'execution-card',
                        referenceId: latestExecutionCard.id,
                        capturedAt: latestExecutionCard.updated_at,
                        runId: latestExecutionCard.source_workflow_run_id,
                        gitBranch: latestApprovedProposal?.git_branch ?? null,
                        gitCommit: latestApprovedProposal?.git_commit ?? null,
                    }
                    : null,
            })
            if (latestProjectScope?.conversationId !== snapshot.conversation_id) {
                void persistProjectState(projectPath, {
                    active_conversation_id: snapshot.conversation_id,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        }

        if (latestApprovedProposal?.git_branch || latestApprovedProposal?.git_commit) {
            setProjectGitMetadata((current) => ({
                ...current,
                [projectPath]: {
                    branch: latestApprovedProposal.git_branch ?? current[projectPath]?.branch ?? null,
                    commit: latestApprovedProposal.git_commit ?? current[projectPath]?.commit ?? null,
                },
            }))
        }
    }

    const applyConversationStreamEvent = (
        projectPath: string,
        event: ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse,
        source = 'unknown',
    ) => {
        debugProjectChat('apply conversation stream event', {
            source,
            projectPath,
            eventType: event.type,
            conversationId: event.conversation_id,
        })
        const { cache, snapshot } = applyConversationStreamEventToCache(
            conversationCacheRef.current,
            projectPath,
            event as ConversationStreamEvent,
        )
        commitConversationCache(cache)
        if (snapshot) {
            debugProjectChat('apply merged stream snapshot', {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                turnCount: snapshot.turns.length,
            })
        }
    }

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
        let isCancelled = false

        const loadProjectRegistry = async () => {
            try {
                const projects = await fetchProjectRegistryValidated()
                if (isCancelled) {
                    return
                }
                hydrateProjectRegistry(projects.map(toHydratedProjectRecord))
                setPanelError(null)
            } catch (error) {
                if (isCancelled) {
                    return
                }
                setPanelError(extractApiErrorMessage(error, 'Unable to load available projects.'))
            }
        }

        void loadProjectRegistry()
        return () => {
            isCancelled = true
        }
    }, [hydrateProjectRegistry])

    useEffect(() => {
        const projectPathsToFetch = projects
            .map((project) => project.directoryPath)
            .filter((projectPath) => !(projectPath in projectGitMetadata))
        if (projectPathsToFetch.length === 0) {
            return
        }

        let isCancelled = false
        const loadBranches = async () => {
            const entries = await Promise.all(
                projectPathsToFetch.map(async (projectPath) => {
                    try {
                        const metadata = await fetchProjectMetadataValidated(projectPath)
                        return [
                            projectPath,
                            {
                                branch: asProjectGitMetadataField(metadata.branch),
                                commit: asProjectGitMetadataField(metadata.commit),
                            },
                        ] as const
                    } catch {
                        return [projectPath, { ...EMPTY_PROJECT_GIT_METADATA }] as const
                    }
                }),
            )

            if (isCancelled) {
                return
            }

            setProjectGitMetadata((current) => {
                const next = { ...current }
                entries.forEach(([projectPath, metadata]) => {
                    next[projectPath] = metadata
                })
                return next
            })
        }

        void loadBranches()
        return () => {
            isCancelled = true
        }
    }, [projectGitMetadata, projects])

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
    }, [activeConversationId, activeProjectPath])

    useEffect(() => {
        resetComposer()
        setPanelError(null)
    }, [activeProjectPath])

    useEffect(() => {
        setExpandedProposalChanges({})
    }, [activeProjectPath, latestSpecEditProposalId])

    useEffect(() => {
        setExpandedToolCalls({})
    }, [activeConversationId, activeProjectPath])

    useEffect(() => {
        setExpandedThinkingEntries({})
    }, [activeConversationId, activeProjectPath])

    useEffect(() => {
        if (!projectDirectoryPickerInputRef.current) {
            return
        }
        projectDirectoryPickerInputRef.current.setAttribute('webkitdirectory', '')
        projectDirectoryPickerInputRef.current.setAttribute('directory', '')
    }, [])

    useEffect(() => {
        if (!isConversationPinnedToBottom) {
            return
        }
        const frame = window.requestAnimationFrame(() => {
            const node = conversationBodyRef.current
            if (!node) {
                return
            }
            node.scrollTop = node.scrollHeight
        })
        return () => {
            window.cancelAnimationFrame(frame)
        }
    }, [activeConversationHistory, activeProjectPath, conversationBodyRef, isConversationPinnedToBottom])

    const fetchProjectGitMetadata = async (
        projectPath: string,
    ): Promise<{ metadata: ProjectGitMetadata; error?: string }> => {
        try {
            const payload = await fetchProjectMetadataValidated(projectPath)
            return {
                metadata: {
                    branch: asProjectGitMetadataField(payload.branch),
                    commit: asProjectGitMetadataField(payload.commit),
                },
            }
        } catch (err) {
            let message = 'Unable to verify project Git state.'
            if (err instanceof ApiHttpError && err.detail) {
                message = err.detail
            }
            return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: message }
        }
    }

    const ensureProjectGitRepository = async (projectPath: string): Promise<ProjectGitMetadata | null> => {
        const { metadata, error } = await fetchProjectGitMetadata(projectPath)
        setProjectGitMetadata((current) => ({ ...current, [projectPath]: metadata }))
        if (error) {
            setProjectRegistrationError(error)
            return null
        }
        if (!metadata.branch && !metadata.commit) {
            setProjectRegistrationError('Project directory must be a Git repository.')
            return null
        }
        return metadata
    }

    const registerProjectFromPath = async (rawProjectPath: string) => {
        const validation = resolveProjectPathValidation(rawProjectPath, projectRegistry)
        if (!validation.ok || !validation.normalizedPath) {
            setProjectRegistrationError(validation.error ?? 'Project directory path is required.')
            return
        }
        const normalizedProjectPath = validation.normalizedPath
        const gitMetadata = await ensureProjectGitRepository(normalizedProjectPath)
        if (!gitMetadata) {
            return
        }
        const result = registerProject(normalizedProjectPath)
        if (!result.ok) {
            setProjectRegistrationError(result.error ?? 'Unable to register the project.')
            return
        }
        try {
            const projectRecord = await registerProjectValidated(normalizedProjectPath)
            upsertProjectRegistryEntry(toHydratedProjectRecord(projectRecord))
        } catch (error) {
            useStore.setState((state) => {
                const nextProjectRegistry = { ...state.projectRegistry }
                const nextProjectSessionStates = { ...state.projectSessionsByPath }
                delete nextProjectRegistry[normalizedProjectPath]
                delete nextProjectSessionStates[normalizedProjectPath]
                const nextActiveProjectPath = state.activeProjectPath === normalizedProjectPath ? null : state.activeProjectPath
                return {
                    projectRegistry: nextProjectRegistry,
                    projectSessionsByPath: nextProjectSessionStates,
                    activeProjectPath: nextActiveProjectPath,
                    activeFlow: state.activeFlow,
                    selectedRunId: nextActiveProjectPath ? state.selectedRunId : null,
                    workingDir: nextActiveProjectPath ? state.workingDir : './test-app',
                }
            })
            setProjectRegistrationError(extractApiErrorMessage(error, 'Unable to register the project.'))
            return
        }
        if (result.ok) {
            setProjectRegistrationError(null)
        }
    }

    const onOpenProjectDirectoryChooser = async () => {
        clearProjectRegistrationError()
        try {
            const selection = await pickProjectDirectoryValidated()
            if (selection.status === 'canceled') {
                return
            }
            await registerProjectFromPath(selection.directory_path)
            return
        } catch (error) {
            const canUseBrowserFallback = error instanceof ApiHttpError
                && [404, 405, 501, 503].includes(error.status)
                && projectDirectoryPickerInputRef.current
            if (!canUseBrowserFallback) {
                setProjectRegistrationError(extractApiErrorMessage(error, 'Directory picker is unavailable.'))
                return
            }
        }
        if (!projectDirectoryPickerInputRef.current) {
            setProjectRegistrationError('Directory picker is unavailable.')
            return
        }
        projectDirectoryPickerInputRef.current.value = ''
        projectDirectoryPickerInputRef.current.click()
    }

    const onProjectDirectorySelected = (event: ChangeEvent<HTMLInputElement>) => {
        const files = event.target.files
        const selectedProjectPath = deriveProjectPathFromDirectorySelection(files)
        event.target.value = ''
        if (!selectedProjectPath) {
            setProjectRegistrationError(
                'Unable to resolve an absolute project path from the selected directory.',
            )
            return
        }
        void registerProjectFromPath(selectedProjectPath)
    }

    const onActivateProject = async (projectPath: string) => {
        if (!projectPath) {
            return
        }
        if (projectPath === activeProjectPath) {
            setActiveProjectPath(projectPath)
            return
        }
        const gitMetadata = await ensureProjectGitRepository(projectPath)
        if (!gitMetadata) {
            return
        }
        setProjectRegistrationError(null)
        setActiveProjectPath(projectPath)
        void persistProjectState(projectPath, {
            last_accessed_at: new Date().toISOString(),
        })
    }

    const onCreateConversationThread = () => {
        if (!activeProjectPath) {
            return
        }
        const now = new Date().toISOString()
        const conversationId = buildProjectConversationId(activeProjectPath)
        setPanelError(null)
        setConversationSummaryList(activeProjectPath, [
            {
                conversation_id: conversationId,
                conversation_handle: '',
                project_path: activeProjectPath,
                title: 'New thread',
                created_at: now,
                updated_at: now,
                last_message_preview: null,
            },
            ...(conversationCacheRef.current.summariesByProjectPath[activeProjectPath] || []),
        ])
        activateConversationThread(activeProjectPath, conversationId, 'create-thread')
    }

    const onSelectConversationThread = (conversationId: string) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        activateConversationThread(activeProjectPath, conversationId, 'select-thread')
        const cachedSnapshot = conversationCacheRef.current.snapshotsByConversationId[conversationId]
        if (cachedSnapshot) {
            applyConversationSnapshot(activeProjectPath, cachedSnapshot, 'thread-cache')
        }
    }

    const onDeleteConversationThread = async (conversationId: string, title: string) => {
        if (!activeProjectPath) {
            return
        }
        if (typeof window !== 'undefined' && !window.confirm(`Delete thread "${title}"?`)) {
            return
        }
        setPanelError(null)
        setPendingDeleteConversationId(conversationId)
        try {
            await deleteConversationValidated(conversationId, activeProjectPath)
            commitConversationCache((current) => removeConversationFromCache(current, conversationId))
            const localRemainingSummaries = (
                conversationCacheRef.current.summariesByProjectPath[activeProjectPath] || []
            ).filter((entry) => entry.conversation_id !== conversationId)
            setConversationSummaryList(activeProjectPath, localRemainingSummaries)

            let remainingSummaries = localRemainingSummaries
            try {
                remainingSummaries = await fetchProjectConversationListValidated(activeProjectPath)
                setConversationSummaryList(activeProjectPath, remainingSummaries)
            } catch {
                // Keep the local optimistic removal if the follow-up refresh fails.
            }

            if (activeConversationId === conversationId) {
                const fallbackConversationId = remainingSummaries[0]?.conversation_id || null
                resetComposer()
                setConversationId(fallbackConversationId)
                if (fallbackConversationId) {
                    updateProjectSessionState(activeProjectPath, {
                        conversationId: fallbackConversationId,
                    })
                }
                void persistProjectState(activeProjectPath, {
                    active_conversation_id: fallbackConversationId,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        } catch (error) {
            const message = extractApiErrorMessage(error, 'Unable to delete the thread.')
            setPanelError(message)
            appendLocalProjectEvent(`Thread deletion failed: ${message}`)
        } finally {
            setPendingDeleteConversationId(null)
        }
    }

    const onDeleteProject = async (projectPath: string) => {
        const projectLabel = formatProjectListLabel(projectPath)
        if (
            typeof window !== 'undefined'
            && !window.confirm(
                `Remove project "${projectLabel}" from Spark? This deletes its local threads, workflow history, and runs, but does not delete the project files.`,
            )
        ) {
            return
        }

        setPanelError(null)
        setPendingDeleteProjectPath(projectPath)
        try {
            await deleteProjectValidated(projectPath)

            setProjectGitMetadata((current) => {
                const next = { ...current }
                delete next[projectPath]
                return next
            })
            commitConversationCache((current) => removeProjectFromCache(current, projectPath))

            const fallbackProjectPath = activeProjectPath === projectPath
                ? orderedProjects.find((project) => project.directoryPath !== projectPath)?.directoryPath || null
                : null
            removeProject(projectPath, fallbackProjectPath)
        } catch (error) {
            const message = extractApiErrorMessage(error, 'Unable to remove the project.')
            setPanelError(message)
            appendLocalProjectEvent(`Project removal failed: ${message}`)
        } finally {
            setPendingDeleteProjectPath(null)
        }
    }

    const ensureConversationId = () => {
        if (!activeProjectPath) {
            return null
        }
        if (activeConversationId) {
            return activeConversationId
        }
        const conversationId = buildProjectConversationId(activeProjectPath)
        activateConversationThread(activeProjectPath, conversationId, 'ensure-conversation')
        return conversationId
    }

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
        setViewMode('execution')
    }

    const toggleProposalChangeExpanded = (changeKey: string) => {
        setExpandedProposalChanges((current) => ({
            ...current,
            [changeKey]: !current[changeKey],
        }))
    }

    const toggleToolCallExpanded = (toolCallId: string) => {
        setExpandedToolCalls((current) => ({
            ...current,
            [toolCallId]: !current[toolCallId],
        }))
    }

    const toggleThinkingEntryExpanded = (entryId: string) => {
        setExpandedThinkingEntries((current) => ({
            ...current,
            [entryId]: !current[entryId],
        }))
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
            projectDirectoryPickerInputRef,
            projectRegistrationError,
            orderedProjects,
            activeProjectPath,
            activeConversationId,
            activeProjectConversationSummaries,
            pendingDeleteProjectPath,
            pendingDeleteConversationId,
            activeProjectEventLog,
            isHomeSidebarResizing,
            onOpenProjectDirectoryChooser,
            onProjectDirectorySelected,
            onActivateProject,
            onDeleteProject,
            onCreateConversationThread,
            onSelectConversationThread,
            onDeleteConversationThread,
            onHomeSidebarResizePointerDown,
            onHomeSidebarResizeKeyDown,
            formatProjectListLabel,
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
