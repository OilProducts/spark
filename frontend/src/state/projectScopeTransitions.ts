import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import { buildRunsScopeKey } from './runsSessionScope'
import {
    DEFAULT_WORKING_DIRECTORY,
    pushRecentProjectPath,
    resolveProjectSessionState,
    resolveViewModeForProjectScope,
} from './store-helpers'
import type {
    AppState,
    HydratedProjectRecord,
    ProjectSessionState,
    RegisteredProject,
    RuntimeStatus,
    ViewMode,
} from './store-types'

type ProjectScopeTransitionState = Pick<
    AppState,
    | 'projectRegistry'
    | 'projectSessionsByPath'
    | 'recentProjectPaths'
    | 'activeProjectPath'
    | 'viewMode'
    | 'workingDir'
    | 'activeFlow'
    | 'executionFlow'
    | 'executionContinuation'
    | 'selectedRunId'
    | 'selectedRunRecord'
    | 'selectedRunCompletedNodes'
    | 'selectedRunStatusSync'
    | 'selectedRunStatusError'
    | 'selectedRunStatusFetchedAtMs'
    | 'runtimeStatus'
    | 'runtimeOutcome'
    | 'runtimeOutcomeReasonCode'
    | 'runtimeOutcomeReasonMessage'
    | 'nodeStatuses'
    | 'humanGate'
    | 'selectedNodeId'
    | 'selectedEdgeId'
    | 'graphAttrs'
    | 'graphAttrErrors'
    | 'graphAttrsUserEditVersion'
    | 'diagnostics'
    | 'nodeDiagnostics'
    | 'edgeDiagnostics'
    | 'hasValidationErrors'
    | 'saveState'
    | 'saveStateVersion'
    | 'saveErrorMessage'
    | 'saveErrorKind'
    | 'runsListSession'
    | 'runDetailSessionsByRunId'
> & Partial<Pick<
    AppState,
    | 'homeConversationCache'
    | 'homeConversationSessionsById'
    | 'homeThreadSummariesStatusByProjectPath'
    | 'homeThreadSummariesErrorByProjectPath'
    | 'homeProjectSessionsByPath'
    | 'homeProjectGitMetadataByPath'
>>

const pathBelongsToProject = (path: string | null | undefined, projectPath: string) => (
    typeof path === 'string'
    && (
        path === projectPath
        || path.startsWith(`${projectPath}/`)
    )
)

const runBelongsToProject = (
    run:
        | {
            project_path?: string | null
            working_directory?: string | null
        }
        | null
        | undefined,
    projectPath: string,
) => (
    Boolean(run) && (
        pathBelongsToProject(run?.project_path, projectPath)
        || pathBelongsToProject(run?.working_directory, projectPath)
    )
)

const pruneRunsSessionsForProject = (
    state: Pick<AppState, 'runsListSession' | 'runDetailSessionsByRunId'>,
    projectPath: string,
) => {
    const removedScopeKey = buildRunsScopeKey('active', projectPath)
    const removedRunIds = new Set<string>()

    state.runsListSession.runs.forEach((run) => {
        if (runBelongsToProject(run, projectPath)) {
            removedRunIds.add(run.run_id)
        }
    })

    Object.entries(state.runDetailSessionsByRunId).forEach(([runId, session]) => {
        if (runBelongsToProject(session.summaryRecord, projectPath)) {
            removedRunIds.add(runId)
        }
    })

    return {
        runsListSession: {
            ...state.runsListSession,
            selectedRunIdByScopeKey: Object.fromEntries(
                Object.entries(state.runsListSession.selectedRunIdByScopeKey).filter(([scopeKey, runId]) => (
                    scopeKey !== removedScopeKey && !removedRunIds.has(runId ?? '')
                )),
            ),
        },
        runDetailSessionsByRunId: Object.fromEntries(
            Object.entries(state.runDetailSessionsByRunId).filter(([runId]) => !removedRunIds.has(runId)),
        ),
    }
}

const preserveEditorSession = (state: AppState) => ({
    activeFlow: state.activeFlow,
    selectedNodeId: state.selectedNodeId,
    selectedEdgeId: state.selectedEdgeId,
    graphAttrs: state.graphAttrs,
    graphAttrErrors: state.graphAttrErrors,
    graphAttrsUserEditVersion: state.graphAttrsUserEditVersion,
    diagnostics: state.diagnostics,
    nodeDiagnostics: state.nodeDiagnostics,
    edgeDiagnostics: state.edgeDiagnostics,
    hasValidationErrors: state.hasValidationErrors,
    saveState: state.saveState,
    saveStateVersion: state.saveStateVersion,
    saveErrorMessage: state.saveErrorMessage,
    saveErrorKind: state.saveErrorKind,
})

const preserveExecutionSession = (state: AppState) => ({
    executionFlow: state.executionFlow,
    executionContinuation: state.executionContinuation,
})

const preserveRunInspectionState = (state: AppState) => ({
    selectedRunId: state.selectedRunId,
    selectedRunRecord: state.selectedRunRecord,
    selectedRunCompletedNodes: state.selectedRunCompletedNodes,
    selectedRunStatusSync: state.selectedRunStatusSync,
    selectedRunStatusError: state.selectedRunStatusError,
    selectedRunStatusFetchedAtMs: state.selectedRunStatusFetchedAtMs,
    runtimeStatus: state.runtimeStatus,
    runtimeOutcome: state.runtimeOutcome,
    runtimeOutcomeReasonCode: state.runtimeOutcomeReasonCode,
    runtimeOutcomeReasonMessage: state.runtimeOutcomeReasonMessage,
    nodeStatuses: state.nodeStatuses,
    humanGate: state.humanGate,
})

const resetRunInspectionState = (runtimeStatus: RuntimeStatus = 'idle') => ({
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle' as const,
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    runtimeStatus,
    runtimeOutcome: null,
    runtimeOutcomeReasonCode: null,
    runtimeOutcomeReasonMessage: null,
    nodeStatuses: {},
    humanGate: null,
})

const resolveForegroundRunInspectionState = (
    state: Pick<AppState, 'runsListSession' | 'runDetailSessionsByRunId'>,
    activeProjectPath: string | null,
) => {
    const scopeKey = buildRunsScopeKey(state.runsListSession.scopeMode, activeProjectPath)
    const selectedRunId = state.runsListSession.selectedRunIdByScopeKey[scopeKey] ?? null
    const selectedRunSession = selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? null : null

    return {
        selectedRunId,
        selectedRunRecord: selectedRunSession?.summaryRecord ?? null,
        selectedRunCompletedNodes: selectedRunSession?.completedNodesSnapshot ?? [],
        selectedRunStatusFetchedAtMs: selectedRunSession?.statusFetchedAtMs ?? null,
    }
}

const buildRegisteredProject = (project: HydratedProjectRecord): RegisteredProject | null => {
    const normalizedPath = normalizeProjectPath(project.directoryPath)
    if (!normalizedPath || !isAbsoluteProjectPath(normalizedPath)) {
        return null
    }
    return {
        directoryPath: normalizedPath,
        isFavorite: project.isFavorite === true,
        lastAccessedAt: typeof project.lastAccessedAt === 'string' ? project.lastAccessedAt : null,
        ...(typeof project.executionProfileId === 'string' ? { executionProfileId: project.executionProfileId } : {}),
    }
}

const updateProjectSessionRegistry = (
    projectSessionsByPath: Record<string, ProjectSessionState>,
    projectPath: string,
    activeConversationId: string | null | undefined,
) => ({
    ...projectSessionsByPath,
    [projectPath]: resolveProjectSessionState(
        {
            ...projectSessionsByPath[projectPath],
            conversationId:
                typeof activeConversationId === 'string'
                    ? activeConversationId
                    : projectSessionsByPath[projectPath]?.conversationId ?? null,
        },
        projectPath,
    ),
})

export const buildHydrateProjectRegistryTransition = (
    state: AppState,
    projects: HydratedProjectRecord[],
): ProjectScopeTransitionState => {
    const nextProjectRegistry: Record<string, RegisteredProject> = {}
    let nextProjectSessionStates = { ...state.projectSessionsByPath }

    projects.forEach((project) => {
        const registeredProject = buildRegisteredProject(project)
        if (!registeredProject) {
            return
        }
        nextProjectRegistry[registeredProject.directoryPath] = registeredProject
        nextProjectSessionStates = updateProjectSessionRegistry(
            nextProjectSessionStates,
            registeredProject.directoryPath,
            project.activeConversationId,
        )
    })

    const nextActiveProjectPath =
        state.activeProjectPath && nextProjectRegistry[state.activeProjectPath] ? state.activeProjectPath : null
    const nextActiveProjectScope = nextActiveProjectPath
        ? resolveProjectSessionState(nextProjectSessionStates[nextActiveProjectPath], nextActiveProjectPath)
        : null
    const nextViewMode = resolveViewModeForProjectScope(state.viewMode)
    const nextForegroundRunState = nextActiveProjectPath === state.activeProjectPath
        ? preserveRunInspectionState(state)
        : {
            ...resetRunInspectionState(state.runtimeStatus),
            ...resolveForegroundRunInspectionState(state, nextActiveProjectPath),
        }

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        activeProjectPath: nextActiveProjectPath,
        viewMode: nextViewMode,
        ...nextForegroundRunState,
        workingDir: nextActiveProjectPath
            ? nextActiveProjectScope?.workingDir || DEFAULT_WORKING_DIRECTORY
            : DEFAULT_WORKING_DIRECTORY,
        recentProjectPaths: nextActiveProjectPath
            ? pushRecentProjectPath(state.recentProjectPaths, nextActiveProjectPath)
            : state.recentProjectPaths,
        runsListSession: state.runsListSession,
        runDetailSessionsByRunId: state.runDetailSessionsByRunId,
    }
}

export const buildRemoveProjectTransition = (
    state: AppState,
    directoryPath: string,
    nextActiveProjectPath: string | null = null,
): ProjectScopeTransitionState | null => {
    const normalizedPath = normalizeProjectPath(directoryPath)
    if (!normalizedPath || !state.projectRegistry[normalizedPath]) {
        return null
    }

    const nextProjectRegistry = { ...state.projectRegistry }
    delete nextProjectRegistry[normalizedPath]

    const nextProjectSessionStates = { ...state.projectSessionsByPath }
    delete nextProjectSessionStates[normalizedPath]

    const nextHomeProjectSessionsByPath = { ...state.homeProjectSessionsByPath }
    delete nextHomeProjectSessionsByPath[normalizedPath]

    const nextHomeThreadSummariesStatusByProjectPath = { ...state.homeThreadSummariesStatusByProjectPath }
    delete nextHomeThreadSummariesStatusByProjectPath[normalizedPath]

    const nextHomeThreadSummariesErrorByProjectPath = { ...state.homeThreadSummariesErrorByProjectPath }
    delete nextHomeThreadSummariesErrorByProjectPath[normalizedPath]

    const nextHomeProjectGitMetadataByPath = { ...state.homeProjectGitMetadataByPath }
    delete nextHomeProjectGitMetadataByPath[normalizedPath]

    const nextHomeSummariesByProjectPath = { ...state.homeConversationCache.summariesByProjectPath }
    const removedConversationIds = new Set(
        (nextHomeSummariesByProjectPath[normalizedPath] ?? []).map((summary) => summary.conversation_id),
    )
    delete nextHomeSummariesByProjectPath[normalizedPath]

    const nextHomeConversationsById = { ...state.homeConversationCache.conversationsById }
    Object.entries(state.homeConversationCache.conversationsById).forEach(([conversationId, conversation]) => {
        if (conversation.project_path === normalizedPath) {
            removedConversationIds.add(conversationId)
            delete nextHomeConversationsById[conversationId]
        }
    })
    const nextHomeConversationSessionsById = { ...state.homeConversationSessionsById }
    removedConversationIds.forEach((conversationId) => {
        delete nextHomeConversationSessionsById[conversationId]
    })

    const normalizedFallbackPath = nextActiveProjectPath ? normalizeProjectPath(nextActiveProjectPath) : null
    const derivedFallbackPath =
        normalizedFallbackPath && nextProjectRegistry[normalizedFallbackPath]
            ? normalizedFallbackPath
            : state.recentProjectPaths.find((path) => path !== normalizedPath && Boolean(nextProjectRegistry[path]))
                || Object.keys(nextProjectRegistry)[0]
                || null
    const nextResolvedActiveProjectPath =
        state.activeProjectPath === normalizedPath ? derivedFallbackPath : state.activeProjectPath
    const nextActiveProjectScope = nextResolvedActiveProjectPath
        ? resolveProjectSessionState(
            nextProjectSessionStates[nextResolvedActiveProjectPath],
            nextResolvedActiveProjectPath,
        )
        : null
    const nextViewMode = resolveViewModeForProjectScope(state.viewMode)
    const removedActiveProject = state.activeProjectPath === normalizedPath
    const nextRunsSessions = pruneRunsSessionsForProject(state, normalizedPath)
    const nextForegroundRunState = removedActiveProject
        ? {
            ...resetRunInspectionState(),
            ...resolveForegroundRunInspectionState(state, nextResolvedActiveProjectPath),
        }
        : preserveRunInspectionState(state)

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        recentProjectPaths: state.recentProjectPaths.filter((path) => path !== normalizedPath),
        activeProjectPath: nextResolvedActiveProjectPath,
        viewMode: nextViewMode,
        ...nextForegroundRunState,
        ...nextRunsSessions,
        workingDir: nextResolvedActiveProjectPath
            ? nextActiveProjectScope?.workingDir || DEFAULT_WORKING_DIRECTORY
            : DEFAULT_WORKING_DIRECTORY,
        homeConversationCache: {
            conversationsById: nextHomeConversationsById,
            summariesByProjectPath: nextHomeSummariesByProjectPath,
        },
        homeConversationSessionsById: nextHomeConversationSessionsById,
        homeThreadSummariesStatusByProjectPath: nextHomeThreadSummariesStatusByProjectPath,
        homeThreadSummariesErrorByProjectPath: nextHomeThreadSummariesErrorByProjectPath,
        homeProjectSessionsByPath: nextHomeProjectSessionsByPath,
        homeProjectGitMetadataByPath: nextHomeProjectGitMetadataByPath,
    }
}

export const buildSetActiveProjectTransition = (
    state: AppState,
    projectPath: string | null,
): ProjectScopeTransitionState | null => {
    const normalizedProjectPath = typeof projectPath === 'string' ? normalizeProjectPath(projectPath) : null
    if (projectPath !== null && (!normalizedProjectPath || !isAbsoluteProjectPath(normalizedProjectPath))) {
        return null
    }

    const nextProjectPath = normalizedProjectPath
    const isProjectSwitch = nextProjectPath !== state.activeProjectPath
    const nextProjectSessionStates = { ...state.projectSessionsByPath }
    const nextProjectRegistry = { ...state.projectRegistry }
    const nextProjectScope = nextProjectPath
        ? resolveProjectSessionState(nextProjectSessionStates[nextProjectPath], nextProjectPath)
        : null

    if (nextProjectPath && nextProjectScope) {
        nextProjectSessionStates[nextProjectPath] = nextProjectScope
    }

    if (nextProjectPath && nextProjectRegistry[nextProjectPath]) {
        nextProjectRegistry[nextProjectPath] = {
            ...nextProjectRegistry[nextProjectPath],
            lastAccessedAt: new Date().toISOString(),
        }
    }

    const nextViewMode = resolveViewModeForProjectScope(state.viewMode)
    const nextForegroundRunState = isProjectSwitch
        ? {
            ...resetRunInspectionState(),
            ...resolveForegroundRunInspectionState(state, nextProjectPath),
        }
        : preserveRunInspectionState(state)

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        recentProjectPaths: pushRecentProjectPath(state.recentProjectPaths, nextProjectPath),
        activeProjectPath: nextProjectPath,
        viewMode: nextViewMode,
        workingDir: nextProjectPath && nextProjectScope ? nextProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
        ...nextForegroundRunState,
        runsListSession: state.runsListSession,
        runDetailSessionsByRunId: state.runDetailSessionsByRunId,
    }
}

export const buildRegisterProjectTransition = (
    state: AppState,
    normalizedPath: string,
) => {
    const nextActiveProjectPath = state.activeProjectPath ?? normalizedPath
    const shouldActivateNewProject = !state.activeProjectPath
    const nowIso = shouldActivateNewProject ? new Date().toISOString() : null
    const nextProjectSessionStates = { ...state.projectSessionsByPath }
    const nextProjectRegistry = {
        ...state.projectRegistry,
        [normalizedPath]: {
            directoryPath: normalizedPath,
            isFavorite: false,
            lastAccessedAt: nowIso,
        },
    }
    nextProjectSessionStates[normalizedPath] = resolveProjectSessionState(
        nextProjectSessionStates[normalizedPath],
        normalizedPath,
    )
    const nextActiveProjectScope = resolveProjectSessionState(
        nextProjectSessionStates[nextActiveProjectPath],
        nextActiveProjectPath,
    )

    return {
        projectRegistry: nextProjectRegistry,
        recentProjectPaths: shouldActivateNewProject
            ? pushRecentProjectPath(state.recentProjectPaths, normalizedPath)
            : state.recentProjectPaths,
        projectRegistrationError: null,
        activeProjectPath: nextActiveProjectPath,
        projectSessionsByPath: nextProjectSessionStates,
        activeFlow: state.activeFlow,
        executionFlow: state.activeProjectPath ? state.executionFlow : null,
        selectedRunId: state.activeProjectPath ? state.selectedRunId : null,
        selectedRunRecord: state.activeProjectPath ? state.selectedRunRecord : null,
        selectedRunCompletedNodes: state.activeProjectPath ? state.selectedRunCompletedNodes : [],
        selectedRunStatusSync: state.activeProjectPath ? state.selectedRunStatusSync : 'idle',
        selectedRunStatusError: state.activeProjectPath ? state.selectedRunStatusError : null,
        selectedRunStatusFetchedAtMs: state.activeProjectPath ? state.selectedRunStatusFetchedAtMs : null,
        workingDir: state.activeProjectPath ? state.workingDir : nextActiveProjectScope.workingDir,
    }
}

export const saveProjectScopeRouteState = (state: Pick<ProjectScopeTransitionState, 'viewMode' | 'activeProjectPath'>) => ({
    viewMode: state.viewMode as ViewMode,
    activeProjectPath: state.activeProjectPath,
})
