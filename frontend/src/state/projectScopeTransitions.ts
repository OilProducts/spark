import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
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
    | 'selectedRunId'
    | 'runtimeStatus'
    | 'runtimeOutcome'
    | 'runtimeOutcomeReasonCode'
    | 'runtimeOutcomeReasonMessage'
    | 'nodeStatuses'
    | 'humanGate'
    | 'logs'
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
>

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
})

const preserveRunInspectionState = (state: AppState) => ({
    selectedRunId: state.selectedRunId,
    runtimeStatus: state.runtimeStatus,
    runtimeOutcome: state.runtimeOutcome,
    runtimeOutcomeReasonCode: state.runtimeOutcomeReasonCode,
    runtimeOutcomeReasonMessage: state.runtimeOutcomeReasonMessage,
    nodeStatuses: state.nodeStatuses,
    humanGate: state.humanGate,
    logs: state.logs,
})

const resetRunInspectionState = (runtimeStatus: RuntimeStatus = 'idle') => ({
    selectedRunId: null,
    runtimeStatus,
    runtimeOutcome: null,
    runtimeOutcomeReasonCode: null,
    runtimeOutcomeReasonMessage: null,
    nodeStatuses: {},
    humanGate: null,
    logs: [],
})

const buildRegisteredProject = (project: HydratedProjectRecord): RegisteredProject | null => {
    const normalizedPath = normalizeProjectPath(project.directoryPath)
    if (!normalizedPath || !isAbsoluteProjectPath(normalizedPath)) {
        return null
    }
    return {
        directoryPath: normalizedPath,
        isFavorite: project.isFavorite === true,
        lastAccessedAt: typeof project.lastAccessedAt === 'string' ? project.lastAccessedAt : null,
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
    const nextViewMode = resolveViewModeForProjectScope(state.viewMode, nextActiveProjectPath)

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        activeProjectPath: nextActiveProjectPath,
        viewMode: nextViewMode,
        selectedRunId: null,
        runtimeStatus: state.runtimeStatus,
        runtimeOutcome: null,
        runtimeOutcomeReasonCode: null,
        runtimeOutcomeReasonMessage: null,
        nodeStatuses: state.nodeStatuses,
        humanGate: state.humanGate,
        logs: state.logs,
        workingDir: nextActiveProjectPath
            ? nextActiveProjectScope?.workingDir || DEFAULT_WORKING_DIRECTORY
            : DEFAULT_WORKING_DIRECTORY,
        recentProjectPaths: nextActiveProjectPath
            ? pushRecentProjectPath(state.recentProjectPaths, nextActiveProjectPath)
            : state.recentProjectPaths,
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
    const nextViewMode = resolveViewModeForProjectScope(state.viewMode, nextResolvedActiveProjectPath)
    const removedActiveProject = state.activeProjectPath === normalizedPath

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        recentProjectPaths: state.recentProjectPaths.filter((path) => path !== normalizedPath),
        activeProjectPath: nextResolvedActiveProjectPath,
        viewMode: nextViewMode,
        selectedRunId: removedActiveProject ? null : state.selectedRunId,
        runtimeStatus: state.runtimeStatus,
        runtimeOutcome: removedActiveProject ? null : state.runtimeOutcome,
        runtimeOutcomeReasonCode: removedActiveProject ? null : state.runtimeOutcomeReasonCode,
        runtimeOutcomeReasonMessage: removedActiveProject ? null : state.runtimeOutcomeReasonMessage,
        nodeStatuses: state.nodeStatuses,
        humanGate: state.humanGate,
        logs: state.logs,
        workingDir: nextResolvedActiveProjectPath
            ? nextActiveProjectScope?.workingDir || DEFAULT_WORKING_DIRECTORY
            : DEFAULT_WORKING_DIRECTORY,
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

    const nextViewMode = resolveViewModeForProjectScope(state.viewMode, nextProjectPath)

    return {
        ...preserveEditorSession(state),
        ...preserveExecutionSession(state),
        projectRegistry: nextProjectRegistry,
        projectSessionsByPath: nextProjectSessionStates,
        recentProjectPaths: pushRecentProjectPath(state.recentProjectPaths, nextProjectPath),
        activeProjectPath: nextProjectPath,
        viewMode: nextViewMode,
        workingDir: nextProjectPath && nextProjectScope ? nextProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
        ...(isProjectSwitch ? resetRunInspectionState() : preserveRunInspectionState(state)),
    }
}

export const saveProjectScopeRouteState = (state: Pick<ProjectScopeTransitionState, 'viewMode' | 'activeProjectPath'>) => ({
    viewMode: state.viewMode as ViewMode,
    activeProjectPath: state.activeProjectPath,
})
