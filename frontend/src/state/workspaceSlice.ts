import { type StateCreator } from 'zustand'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    buildHydrateProjectRegistryTransition,
    buildRegisterProjectTransition,
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

export const createWorkspaceSlice: StateCreator<AppState, [], [], WorkspaceSlice> = (set) => ({
    viewMode: restoredRouteState.viewMode,
    setViewMode: (mode) =>
        set((state) => {
            const nextViewMode = resolveViewModeForProjectScope(mode)
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

            const nextState = buildRegisterProjectTransition(state, normalizedPath)
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextState.activeProjectPath,
            })
            result = {
                ok: true,
                normalizedPath,
            }
            return nextState
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
            const nextHomeProjectSessionsByPath = { ...state.homeProjectSessionsByPath }
            if (nextHomeProjectSessionsByPath[normalizedCurrentPath]) {
                nextHomeProjectSessionsByPath[normalizedNextPath] = nextHomeProjectSessionsByPath[normalizedCurrentPath]
                delete nextHomeProjectSessionsByPath[normalizedCurrentPath]
            }
            const nextHomeThreadSummariesStatusByProjectPath = { ...state.homeThreadSummariesStatusByProjectPath }
            if (normalizedCurrentPath in nextHomeThreadSummariesStatusByProjectPath) {
                nextHomeThreadSummariesStatusByProjectPath[normalizedNextPath] =
                    nextHomeThreadSummariesStatusByProjectPath[normalizedCurrentPath]
                delete nextHomeThreadSummariesStatusByProjectPath[normalizedCurrentPath]
            }
            const nextHomeThreadSummariesErrorByProjectPath = { ...state.homeThreadSummariesErrorByProjectPath }
            if (normalizedCurrentPath in nextHomeThreadSummariesErrorByProjectPath) {
                nextHomeThreadSummariesErrorByProjectPath[normalizedNextPath] =
                    nextHomeThreadSummariesErrorByProjectPath[normalizedCurrentPath]
                delete nextHomeThreadSummariesErrorByProjectPath[normalizedCurrentPath]
            }
            const nextHomeProjectGitMetadataByPath = { ...state.homeProjectGitMetadataByPath }
            if (normalizedCurrentPath in nextHomeProjectGitMetadataByPath) {
                nextHomeProjectGitMetadataByPath[normalizedNextPath] =
                    nextHomeProjectGitMetadataByPath[normalizedCurrentPath]
                delete nextHomeProjectGitMetadataByPath[normalizedCurrentPath]
            }
            const nextHomeConversationCache = {
                conversationsById: Object.fromEntries(
                    Object.entries(state.homeConversationCache.conversationsById).map(([conversationId, conversation]) => ([
                        conversationId,
                        conversation.project_path === normalizedCurrentPath
                            ? {
                                ...conversation,
                                project_path: normalizedNextPath,
                            }
                            : conversation,
                    ])),
                ),
                summariesByProjectPath: {
                    ...state.homeConversationCache.summariesByProjectPath,
                },
            }
            if (normalizedCurrentPath in nextHomeConversationCache.summariesByProjectPath) {
                nextHomeConversationCache.summariesByProjectPath[normalizedNextPath] =
                    nextHomeConversationCache.summariesByProjectPath[normalizedCurrentPath].map((summary) => ({
                        ...summary,
                        project_path: normalizedNextPath,
                    }))
                delete nextHomeConversationCache.summariesByProjectPath[normalizedCurrentPath]
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
                homeConversationCache: nextHomeConversationCache,
                homeThreadSummariesStatusByProjectPath: nextHomeThreadSummariesStatusByProjectPath,
                homeThreadSummariesErrorByProjectPath: nextHomeThreadSummariesErrorByProjectPath,
                homeProjectSessionsByPath: nextHomeProjectSessionsByPath,
                homeProjectGitMetadataByPath: nextHomeProjectGitMetadataByPath,
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
})
