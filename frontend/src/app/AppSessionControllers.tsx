import { useCallback, useEffect, useMemo } from 'react'

import { usePersistProjectState } from '@/features/projects/hooks/usePersistProjectState'
import { useConversationStream } from '@/features/projects/hooks/useConversationStream'
import { useProjectConversationCache } from '@/features/projects/hooks/useProjectConversationCache'
import { useProjectGitMetadata } from '@/features/projects/hooks/useProjectGitMetadata'
import { extractApiErrorMessage } from '@/features/projects/model/projectsHomeState'
import { useRunDetailResources } from '@/features/runs/hooks/useRunDetailResources'
import { useRunsList } from '@/features/runs/hooks/useRunsList'
import type { RunRecord } from '@/features/runs/model/shared'
import { useTriggersList } from '@/features/triggers/hooks/useTriggersList'
import type {
    ConversationSnapshotResponse,
    ConversationSummaryResponse,
} from '@/lib/workspaceClient'
import type {
    ApplyConversationStreamEventResult,
    ConversationStreamEvent,
} from '@/features/projects/model/projectsHomeState'
import { useStore } from '@/store'
import { buildRunsScopeKey, getRunsSelectedRunIdForScope } from '@/state/runsSessionScope'

const completedNodesMatch = (left: string[], right: string[]) => (
    left.length === right.length && left.every((value, index) => value === right[index])
)

const runRecordsMatch = (
    left: RunRecord | null,
    right: RunRecord | null,
) => {
    if (left === right) {
        return true
    }
    if (!left || !right) {
        return false
    }
    return [
        'run_id',
        'flow_name',
        'status',
        'outcome',
        'outcome_reason_code',
        'outcome_reason_message',
        'working_directory',
        'project_path',
        'git_branch',
        'git_commit',
        'spec_id',
        'plan_id',
        'model',
        'started_at',
        'ended_at',
        'last_error',
        'token_usage',
        'current_node',
        'continued_from_run_id',
        'continued_from_node',
        'continued_from_flow_mode',
        'continued_from_flow_name',
        'parent_run_id',
        'parent_node_id',
        'root_run_id',
        'child_invocation_index',
    ].every((key) => left[key as keyof RunRecord] === right[key as keyof RunRecord])
}

type HomeConversationSyncControllerProps = {
    projectPath: string
    conversationId: string | null
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source?: string,
    ) => void
    applyConversationStreamEvent: (
        projectPath: string,
        event: ConversationStreamEvent,
        source?: string,
    ) => ApplyConversationStreamEventResult | undefined
    appendProjectEvent: (projectPath: string, message: string) => void
    setProjectPanelError: (projectPath: string, value: string | null) => void
}

function HomeConversationSyncController({
    projectPath,
    conversationId,
    applyConversationSnapshot,
    applyConversationStreamEvent,
    appendProjectEvent,
    setProjectPanelError,
}: HomeConversationSyncControllerProps) {
    const appendLocalProjectEvent = useCallback((message: string) => {
        appendProjectEvent(projectPath, message)
    }, [appendProjectEvent, projectPath])

    const setPanelError = useCallback((value: string | null) => {
        setProjectPanelError(projectPath, value)
    }, [projectPath, setProjectPanelError])

    useConversationStream({
        activeConversationId: conversationId,
        activeProjectPath: projectPath,
        appendLocalProjectEvent,
        applyConversationSnapshot,
        applyConversationStreamEvent,
        formatErrorMessage: extractApiErrorMessage,
        setPanelError,
    })

    return null
}

type HomeThreadSummariesControllerProps = {
    projectPath: string
    activeConversationId: string | null
    isForegroundProject: boolean
    threadStatus: 'idle' | 'loading' | 'ready' | 'error'
    loadProjectConversationSummaries: (projectPath: string) => Promise<ConversationSummaryResponse[]>
    activateConversationThread: (projectPath: string, conversationId: string) => void
}

function HomeThreadSummariesController({
    projectPath,
    activeConversationId,
    isForegroundProject,
    threadStatus,
    loadProjectConversationSummaries,
    activateConversationThread,
}: HomeThreadSummariesControllerProps) {
    useEffect(() => {
        if (threadStatus !== 'idle') {
            return
        }

        let isCancelled = false

        const loadThreadSummaries = async () => {
            const summaries = await loadProjectConversationSummaries(projectPath)
            if (isCancelled || !isForegroundProject || activeConversationId) {
                return
            }
            const latestConversation = summaries[0] ?? null
            if (latestConversation) {
                activateConversationThread(projectPath, latestConversation.conversation_id)
            }
        }

        void loadThreadSummaries()
        return () => {
            isCancelled = true
        }
    }, [
        activeConversationId,
        activateConversationThread,
        isForegroundProject,
        loadProjectConversationSummaries,
        projectPath,
    ])

    return null
}

export function HomeSessionController() {
    const viewMode = useStore((state) => state.viewMode)
    const projectRegistry = useStore((state) => state.projectRegistry)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectSessionsByPath = useStore((state) => state.projectSessionsByPath)
    const homeConversationCache = useStore((state) => state.homeConversationCache)
    const homeThreadSummariesStatusByProjectPath = useStore((state) => state.homeThreadSummariesStatusByProjectPath)
    const homeThreadSummariesErrorByProjectPath = useStore((state) => state.homeThreadSummariesErrorByProjectPath)
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeProjectGitMetadataByPath = useStore((state) => state.homeProjectGitMetadataByPath)
    const removeHomeProjectSession = useStore((state) => state.removeHomeProjectSession)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const persistProjectState = usePersistProjectState(upsertProjectRegistryEntry)
    const {
        applyConversationSnapshot,
        applyConversationStreamEvent,
        loadProjectConversationSummaries,
    } = useProjectConversationCache({
        persistProjectState,
        projectSessionsByPath,
        updateProjectSessionState,
    })

    const homeProjectPaths = useMemo(() => {
        const nextProjectPaths = new Set<string>()
        const isHomeVisible = viewMode === 'home' || viewMode === 'projects'

        if (isHomeVisible && activeProjectPath) {
            nextProjectPaths.add(activeProjectPath)
        }

        Object.entries(projectSessionsByPath).forEach(([projectPath, session]) => {
            if (session?.conversationId) {
                nextProjectPaths.add(projectPath)
            }
        })
        Object.keys(homeProjectSessionsByPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesStatusByProjectPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.keys(homeConversationCache.summariesByProjectPath).forEach((projectPath) => {
            nextProjectPaths.add(projectPath)
        })
        Object.values(homeConversationCache.conversationsById).forEach((conversation) => {
            nextProjectPaths.add(conversation.project_path)
        })

        return [...nextProjectPaths].filter((projectPath) => Boolean(projectRegistry[projectPath]))
    }, [
        activeProjectPath,
        homeConversationCache,
        homeProjectSessionsByPath,
        homeThreadSummariesStatusByProjectPath,
        projectRegistry,
        projectSessionsByPath,
        viewMode,
    ])

    useProjectGitMetadata({
        projectPaths: homeProjectPaths,
        setProjectRegistrationError: () => {},
    })

    const appendProjectEvent = useCallback((projectPath: string, message: string) => {
        const currentProjectEventLog = useStore.getState().projectSessionsByPath[projectPath]?.projectEventLog ?? []
        updateProjectSessionState(projectPath, {
            projectEventLog: [
                ...currentProjectEventLog,
                {
                    message,
                    timestamp: new Date().toISOString(),
                },
            ],
        })
    }, [updateProjectSessionState])

    const setProjectPanelError = useCallback((projectPath: string, value: string | null) => {
        updateHomeProjectSession(projectPath, { panelError: value })
    }, [updateHomeProjectSession])

    const activateConversationThread = useCallback((projectPath: string, conversationId: string) => {
        updateProjectSessionState(projectPath, { conversationId })
        void persistProjectState(projectPath, {
            active_conversation_id: conversationId,
            last_accessed_at: new Date().toISOString(),
        })
    }, [persistProjectState, updateProjectSessionState])

    useEffect(() => {
        const registeredPaths = new Set(Object.keys(projectRegistry))
        const knownHomeProjectPaths = new Set<string>()

        Object.keys(homeProjectSessionsByPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesStatusByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeThreadSummariesErrorByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeProjectGitMetadataByPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.keys(homeConversationCache.summariesByProjectPath).forEach((projectPath) => {
            knownHomeProjectPaths.add(projectPath)
        })
        Object.values(homeConversationCache.conversationsById).forEach((conversation) => {
            knownHomeProjectPaths.add(conversation.project_path)
        })

        knownHomeProjectPaths.forEach((projectPath) => {
            if (!registeredPaths.has(projectPath)) {
                removeHomeProjectSession(projectPath)
            }
        })
    }, [
        homeConversationCache,
        homeProjectGitMetadataByPath,
        homeProjectSessionsByPath,
        homeThreadSummariesErrorByProjectPath,
        homeThreadSummariesStatusByProjectPath,
        projectRegistry,
        removeHomeProjectSession,
    ])

    const isHomeVisible = viewMode === 'home' || viewMode === 'projects'

    return (
        <>
            {homeProjectPaths.map((projectPath) => (
                <HomeThreadSummariesController
                    key={`${projectPath}:threads`}
                    projectPath={projectPath}
                    activeConversationId={projectSessionsByPath[projectPath]?.conversationId ?? null}
                    isForegroundProject={isHomeVisible && activeProjectPath === projectPath}
                    threadStatus={homeThreadSummariesStatusByProjectPath[projectPath] ?? 'idle'}
                    loadProjectConversationSummaries={loadProjectConversationSummaries}
                    activateConversationThread={activateConversationThread}
                />
            ))}
            {homeProjectPaths.map((projectPath) => (
                <HomeConversationSyncController
                    key={`${projectPath}:${projectSessionsByPath[projectPath]?.conversationId ?? 'none'}`}
                    projectPath={projectPath}
                    conversationId={projectSessionsByPath[projectPath]?.conversationId ?? null}
                    applyConversationSnapshot={applyConversationSnapshot}
                    applyConversationStreamEvent={applyConversationStreamEvent}
                    appendProjectEvent={appendProjectEvent}
                    setProjectPanelError={setProjectPanelError}
                />
            ))}
        </>
    )
}

export function RunsSessionController() {
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const runsListSession = useStore((state) => state.runsListSession)
    const scopeMode = runsListSession.scopeMode
    const globalSelectedRunId = useStore((state) => state.selectedRunId)
    const selectedRunRecord = useStore((state) => state.selectedRunRecord)
    const selectedRunCompletedNodes = useStore((state) => state.selectedRunCompletedNodes)
    const selectedRunStatusFetchedAtMs = useStore((state) => state.selectedRunStatusFetchedAtMs)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setSelectedRunSnapshot = useStore((state) => state.setSelectedRunSnapshot)
    const setRunsSelectedRunIdForScope = useStore((state) => state.setRunsSelectedRunIdForScope)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const scopedSelectedRunId = getRunsSelectedRunIdForScope(runsListSession, activeProjectPath)
    const selectedRunId = scopedSelectedRunId ?? globalSelectedRunId
    const scopeKey = buildRunsScopeKey(scopeMode, activeProjectPath)
    const scopedSelectedRunSession = useStore((state) => (
        scopedSelectedRunId ? state.runDetailSessionsByRunId[scopedSelectedRunId] ?? null : null
    ))

    useEffect(() => {
        const restoreScopedSnapshot = () => {
            setSelectedRunSnapshot({
                record: scopedSelectedRunSession?.summaryRecord ?? null,
                completedNodes: scopedSelectedRunSession?.completedNodesSnapshot ?? [],
                fetchedAtMs: scopedSelectedRunSession?.statusFetchedAtMs ?? null,
            })
        }

        if (selectedRunId && !scopedSelectedRunId) {
            setRunsSelectedRunIdForScope(scopeKey, selectedRunId)
            return
        }
        if (globalSelectedRunId !== scopedSelectedRunId) {
            setSelectedRunId(scopedSelectedRunId)
            restoreScopedSnapshot()
            return
        }
        if (!globalSelectedRunId) {
            if (selectedRunRecord || selectedRunCompletedNodes.length > 0 || selectedRunStatusFetchedAtMs !== null) {
                setSelectedRunSnapshot({ record: null, completedNodes: [], fetchedAtMs: null })
            }
            return
        }
        if (!scopedSelectedRunSession) {
            if (selectedRunRecord && selectedRunRecord.run_id !== selectedRunId) {
                setSelectedRunSnapshot({ record: null, completedNodes: [], fetchedAtMs: null })
            }
            return
        }
        if (!selectedRunRecord || selectedRunRecord.run_id !== selectedRunId) {
            restoreScopedSnapshot()
        }
    }, [
        globalSelectedRunId,
        scopeKey,
        scopedSelectedRunId,
        scopedSelectedRunSession?.completedNodesSnapshot,
        scopedSelectedRunSession?.statusFetchedAtMs,
        scopedSelectedRunSession?.summaryRecord,
        selectedRunCompletedNodes,
        selectedRunId,
        selectedRunRecord,
        selectedRunStatusFetchedAtMs,
        setRunsSelectedRunIdForScope,
        setSelectedRunId,
        setSelectedRunSnapshot,
    ])

    useEffect(() => {
        if (
            !selectedRunId
            || selectedRunId !== scopedSelectedRunId
            || !selectedRunRecord
            || selectedRunRecord.run_id !== selectedRunId
        ) {
            return
        }

        const sessionCompletedNodes = scopedSelectedRunSession?.completedNodesSnapshot ?? []
        const sessionFetchedAtMs = scopedSelectedRunSession?.statusFetchedAtMs ?? null
        if (
            runRecordsMatch(scopedSelectedRunSession?.summaryRecord ?? null, selectedRunRecord)
            && completedNodesMatch(sessionCompletedNodes, selectedRunCompletedNodes)
            && sessionFetchedAtMs === selectedRunStatusFetchedAtMs
        ) {
            return
        }

        updateRunDetailSession(selectedRunId, {
            summaryRecord: selectedRunRecord,
            completedNodesSnapshot: selectedRunCompletedNodes,
            statusFetchedAtMs: selectedRunStatusFetchedAtMs,
        })
    }, [
        scopedSelectedRunId,
        scopedSelectedRunSession?.completedNodesSnapshot,
        scopedSelectedRunSession?.statusFetchedAtMs,
        scopedSelectedRunSession?.summaryRecord,
        selectedRunCompletedNodes,
        selectedRunId,
        selectedRunRecord,
        selectedRunStatusFetchedAtMs,
        updateRunDetailSession,
    ])

    const {
        error,
        isLoading,
        scopedRuns,
        selectedRunSummary,
    } = useRunsList({
        activeProjectPath,
        scopeMode,
        selectedRunId,
        manageSync: true,
    })

    const hasScopedSelectedRun = scopedSelectedRunId
        ? scopedRuns.some((run) => run.run_id === scopedSelectedRunId)
        : false
    const authoritativeSelectedRunRecord = selectedRunRecord?.run_id === selectedRunId
        ? selectedRunRecord
        : null
    const selectedRunSessionRecord = scopedSelectedRunSession?.summaryRecord ?? null
    const selectedRun =
        authoritativeSelectedRunRecord
        ?? (
            selectedRunSessionRecord
            && selectedRunSessionRecord.run_id === scopedSelectedRunId
                ? selectedRunSessionRecord
                : (
                    selectedRunSummary
                    ?? (
                        selectedRunSessionRecord
                        && selectedRunSessionRecord.run_id === scopedSelectedRunId
                        && (isLoading || Boolean(error) || hasScopedSelectedRun || scopedRuns.length === 0)
                            ? selectedRunSessionRecord
                            : null
                    )
                )
        )

    useRunDetailResources({
        selectedRunId: selectedRun?.run_id ?? null,
        manageSync: true,
    })

    return null
}

export function TriggersSessionController() {
    useTriggersList({ manageSync: true })
    return null
}

export function AppSessionControllers() {
    return (
        <>
            <HomeSessionController />
            <RunsSessionController />
            <TriggersSessionController />
        </>
    )
}
