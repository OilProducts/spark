import { type StateCreator } from 'zustand'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    buildHydrateProjectRegistryTransition,
    buildRemoveProjectTransition,
    buildSetActiveProjectTransition,
    saveProjectScopeRouteState,
} from './projectScopeTransitions'
import {
    DEFAULT_WORKING_DIRECTORY,
    loadRouteState,
    pushRecentProjectPath,
    resolveProjectSessionState,
    resolveViewModeForProjectScope,
    saveRouteState,
    selectProjectSessionArtifactState,
} from './store-helpers'
import type {
    AppState,
    ProjectRegistrationResult,
    ProjectSessionState,
    RegisteredProject,
    WorkspaceSlice,
} from './store-types'

const restoredRouteState = loadRouteState()
const initialProjectRegistry: Record<string, RegisteredProject> = {}
const initialProjectSessionStates: Record<string, ProjectSessionState> = restoredRouteState.activeProjectPath
    ? {
        [restoredRouteState.activeProjectPath]: resolveProjectSessionState(
            {},
            restoredRouteState.activeProjectPath,
        ),
    }
    : {}
const restoredProjectScope = restoredRouteState.activeProjectPath
    ? resolveProjectSessionState(
        initialProjectSessionStates[restoredRouteState.activeProjectPath],
        restoredRouteState.activeProjectPath,
    )
    : null

export const initialWorkspaceEditorState = {
    activeFlow: null,
    workingDir: restoredProjectScope ? restoredProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
}

export const createWorkspaceSlice: StateCreator<AppState, [], [], WorkspaceSlice> = (set, get) => ({
    viewMode: restoredRouteState.viewMode,
    setViewMode: (mode) =>
        set((state) => {
            const nextViewMode = resolveViewModeForProjectScope(mode, state.activeProjectPath)
            saveRouteState({
                viewMode: nextViewMode,
                activeProjectPath: state.activeProjectPath,
            })
            return { viewMode: nextViewMode }
        }),
    activeProjectPath: restoredRouteState.activeProjectPath,
    projectRegistry: initialProjectRegistry,
    recentProjectPaths: restoredRouteState.activeProjectPath ? [restoredRouteState.activeProjectPath] : [],
    projectSessionsByPath: initialProjectSessionStates,
    hydrateProjectRegistry: (projects) =>
        set((state) => {
            const nextState = buildHydrateProjectRegistryTransition(state, projects)
            saveRouteState(saveProjectScopeRouteState(nextState))
            return nextState
        }),
    upsertProjectRegistryEntry: (project) =>
        set((state) => {
            const normalizedPath = normalizeProjectPath(project.directoryPath)
            if (!normalizedPath || !isAbsoluteProjectPath(normalizedPath)) {
                return state
            }
            const nextProjectRegistry = {
                ...state.projectRegistry,
                [normalizedPath]: {
                    directoryPath: normalizedPath,
                    isFavorite: project.isFavorite === true,
                    lastAccessedAt: typeof project.lastAccessedAt === 'string' ? project.lastAccessedAt : null,
                },
            }
            const nextProjectSessionStates = {
                ...state.projectSessionsByPath,
                [normalizedPath]: resolveProjectSessionState(
                    {
                        ...state.projectSessionsByPath[normalizedPath],
                        conversationId: typeof project.activeConversationId === 'string'
                            ? project.activeConversationId
                            : state.projectSessionsByPath[normalizedPath]?.conversationId ?? null,
                    },
                    normalizedPath,
                ),
            }
            return {
                projectRegistry: nextProjectRegistry,
                projectSessionsByPath: nextProjectSessionStates,
                recentProjectPaths: pushRecentProjectPath(state.recentProjectPaths, normalizedPath),
            }
        }),
    removeProject: (directoryPath, nextActiveProjectPath = null) =>
        set((state) => {
            const nextState = buildRemoveProjectTransition(state, directoryPath, nextActiveProjectPath)
            if (!nextState) {
                return state
            }
            saveRouteState(saveProjectScopeRouteState(nextState))
            return nextState
        }),
    setActiveProjectPath: (projectPath) =>
        set((state) => {
            const nextState = buildSetActiveProjectTransition(state, projectPath)
            if (!nextState) {
                return state
            }
            saveRouteState(saveProjectScopeRouteState(nextState))
            return nextState
        }),
    projectRegistrationError: null,
    registerProject: (directoryPath) => {
        let result: ProjectRegistrationResult = {
            ok: false,
            error: 'Project directory path is required.',
        }
        set((state) => {
            const normalizedPath = normalizeProjectPath(directoryPath)
            if (!normalizedPath) {
                result = {
                    ok: false,
                    error: 'Project directory path is required.',
                }
                return { projectRegistrationError: result.error }
            }
            if (!isAbsoluteProjectPath(normalizedPath)) {
                result = {
                    ok: false,
                    normalizedPath,
                    error: 'Project directory path must be absolute.',
                }
                return { projectRegistrationError: result.error }
            }

            const duplicate = Boolean(state.projectRegistry[normalizedPath])
            if (duplicate) {
                result = {
                    ok: false,
                    normalizedPath,
                    error: `Project already registered: ${normalizedPath}`,
                }
                return { projectRegistrationError: result.error }
            }

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
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextActiveProjectPath,
            })
            result = {
                ok: true,
                normalizedPath,
            }
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
                workingDir: state.activeProjectPath ? state.workingDir : nextActiveProjectScope.workingDir,
            }
        })
        return result
    },
    updateProjectPath: (currentDirectoryPath, nextDirectoryPath) => {
        let result: ProjectRegistrationResult = {
            ok: false,
            error: 'Project directory path is required.',
        }
        set((state) => {
            const normalizedCurrentPath = normalizeProjectPath(currentDirectoryPath)
            const normalizedNextPath = normalizeProjectPath(nextDirectoryPath)

            if (!normalizedCurrentPath || !state.projectRegistry[normalizedCurrentPath]) {
                result = {
                    ok: false,
                    error: 'Project must already be registered before updating path.',
                }
                return { projectRegistrationError: result.error }
            }
            if (!normalizedNextPath) {
                result = {
                    ok: false,
                    error: 'Project directory path is required.',
                }
                return { projectRegistrationError: result.error }
            }
            if (!isAbsoluteProjectPath(normalizedNextPath)) {
                result = {
                    ok: false,
                    normalizedPath: normalizedNextPath,
                    error: 'Project directory path must be absolute.',
                }
                return { projectRegistrationError: result.error }
            }
            const duplicate = normalizedNextPath !== normalizedCurrentPath && Boolean(state.projectRegistry[normalizedNextPath])
            if (duplicate) {
                result = {
                    ok: false,
                    normalizedPath: normalizedNextPath,
                    error: `Project already registered: ${normalizedNextPath}`,
                }
                return { projectRegistrationError: result.error }
            }
            if (normalizedNextPath === normalizedCurrentPath) {
                result = {
                    ok: true,
                    normalizedPath: normalizedCurrentPath,
                }
                return { projectRegistrationError: null }
            }

            const nextProjectSessionStates = { ...state.projectSessionsByPath }
            const currentWorkspace = resolveProjectSessionState(
                nextProjectSessionStates[normalizedCurrentPath],
                normalizedCurrentPath,
            )
            delete nextProjectSessionStates[normalizedCurrentPath]
            nextProjectSessionStates[normalizedNextPath] = {
                ...currentWorkspace,
                workingDir: currentWorkspace.workingDir === normalizedCurrentPath
                    ? normalizedNextPath
                    : currentWorkspace.workingDir,
            }

            const nextProjectRegistry = { ...state.projectRegistry }
            delete nextProjectRegistry[normalizedCurrentPath]
            nextProjectRegistry[normalizedNextPath] = {
                ...state.projectRegistry[normalizedCurrentPath],
                directoryPath: normalizedNextPath,
            }
            const activeProjectWasUpdated = state.activeProjectPath === normalizedCurrentPath
            const nextActiveProjectPath = activeProjectWasUpdated ? normalizedNextPath : state.activeProjectPath
            const nextWorkingDir = activeProjectWasUpdated && state.workingDir === normalizedCurrentPath
                ? normalizedNextPath
                : state.workingDir
            const nextRecentProjectPaths = state.recentProjectPaths.map((path) =>
                path === normalizedCurrentPath ? normalizedNextPath : path,
            )

            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextActiveProjectPath,
            })
            result = {
                ok: true,
                normalizedPath: normalizedNextPath,
            }
            return {
                projectRegistry: nextProjectRegistry,
                projectSessionsByPath: nextProjectSessionStates,
                activeProjectPath: nextActiveProjectPath,
                workingDir: nextWorkingDir,
                recentProjectPaths: nextRecentProjectPaths,
                projectRegistrationError: null,
            }
        })
        return result
    },
    toggleProjectFavorite: (projectPath) =>
        set((state) => {
            const normalizedPath = normalizeProjectPath(projectPath)
            const project = state.projectRegistry[normalizedPath]
            if (!project) {
                return state
            }
            const nextProjectRegistry = {
                ...state.projectRegistry,
                [normalizedPath]: {
                    ...project,
                    isFavorite: !project.isFavorite,
                },
            }
            return {
                projectRegistry: nextProjectRegistry,
            }
        }),
    setProjectRegistrationError: (error) => set({ projectRegistrationError: error }),
    clearProjectRegistrationError: () => set({ projectRegistrationError: null }),
    activeFlow: initialWorkspaceEditorState.activeFlow,
    setActiveFlow: (flow) =>
        set({ activeFlow: flow }),
    setConversationId: (id) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectSessionStates = { ...state.projectSessionsByPath }
            const scoped = resolveProjectSessionState(nextProjectSessionStates[state.activeProjectPath], state.activeProjectPath)
            nextProjectSessionStates[state.activeProjectPath] = {
                ...scoped,
                conversationId: id,
            }
            return {
                projectSessionsByPath: nextProjectSessionStates,
            }
        }),
    appendProjectEventEntry: (entry) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectSessionStates = { ...state.projectSessionsByPath }
            const scoped = resolveProjectSessionState(nextProjectSessionStates[state.activeProjectPath], state.activeProjectPath)
            nextProjectSessionStates[state.activeProjectPath] = {
                ...scoped,
                projectEventLog: [...scoped.projectEventLog, entry],
            }
            return {
                projectSessionsByPath: nextProjectSessionStates,
            }
        }),
    updateProjectSessionState: (projectPath, patch) =>
        set((state) => {
            const normalizedProjectPath = normalizeProjectPath(projectPath)
            if (!normalizedProjectPath || !isAbsoluteProjectPath(normalizedProjectPath)) {
                return {}
            }
            const nextProjectSessionStates = { ...state.projectSessionsByPath }
            const scoped = resolveProjectSessionState(nextProjectSessionStates[normalizedProjectPath], normalizedProjectPath)
            const nextScopedWorkspace = {
                ...scoped,
                ...patch,
            }
            nextProjectSessionStates[normalizedProjectPath] = nextScopedWorkspace
            const isActiveScope = state.activeProjectPath === normalizedProjectPath
            if (!isActiveScope) {
                return {
                    projectSessionsByPath: nextProjectSessionStates,
                }
            }
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: state.activeProjectPath,
            })
            return {
                projectSessionsByPath: nextProjectSessionStates,
                workingDir: nextScopedWorkspace.workingDir,
            }
        }),
    setSpecId: (id) =>
        set((state) => updateArtifactField(state, 'specId', id)),
    setSpecStatus: (status) =>
        set((state) => updateArtifactField(state, 'specStatus', status)),
    setSpecProvenance: (provenance) =>
        set((state) => updateArtifactField(state, 'specProvenance', provenance)),
    setPlanId: (id) =>
        set((state) => updateArtifactField(state, 'planId', id)),
    setPlanStatus: (status) =>
        set((state) => updateArtifactField(state, 'planStatus', status)),
    setPlanProvenance: (provenance) =>
        set((state) => updateArtifactField(state, 'planProvenance', provenance)),
    getProjectSessionArtifactState: (projectPath) =>
        selectProjectSessionArtifactState(get().projectSessionsByPath, projectPath),
})

const updateArtifactField = <
    Key extends keyof Pick<
        ProjectSessionState,
        'specId' | 'specStatus' | 'specProvenance' | 'planId' | 'planStatus' | 'planProvenance'
    >,
>(
    state: AppState,
    key: Key,
    value: ProjectSessionState[Key],
) => {
    if (!state.activeProjectPath) {
        return {}
    }
    const nextProjectSessionStates = { ...state.projectSessionsByPath }
    const scoped = resolveProjectSessionState(nextProjectSessionStates[state.activeProjectPath], state.activeProjectPath)
    nextProjectSessionStates[state.activeProjectPath] = {
        ...scoped,
        [key]: value,
    }
    return {
        projectSessionsByPath: nextProjectSessionStates,
    }
}
