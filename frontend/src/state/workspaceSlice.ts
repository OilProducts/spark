import { type StateCreator } from 'zustand'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    buildHydrateProjectRegistryTransition,
    buildRegisteredProject,
    buildRegisterProjectTransition,
    buildRemoveProjectTransition,
    buildSetActiveProjectTransition,
} from './projectScopeTransitions'
import {
    DEFAULT_WORKING_DIRECTORY,
    loadRouteState,
    pushRecentProjectPath,
    resolveProjectSessionState,
    resolveViewModeForProjectScope,
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
    activeFlow: restoredRouteState.activeFlow,
    workingDir: restoredProjectScope ? restoredProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
}

/** Runs proceed once open editors allow leaving; a cancelled or blocked leave never runs it. */
export const requestNavigation = (proceed: () => void) => {
    if (window.dispatchEvent(new CustomEvent('spark:before-navigation', { cancelable: true, detail: { proceed } }))) proceed()
}

export const createWorkspaceSlice: StateCreator<AppState, [], [], WorkspaceSlice> = (rawSet, get) => {
    // Leaving Settings or a project page unmounts their editors, so those
    // transitions ask first and unsaved edits can block them. Other views stay
    // mounted and keep their drafts.
    const set = (update: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => {
        const next = typeof update === 'function' ? update(get()) : update
        const proceed = () => rawSet((state) => typeof update === 'function' ? update(state) : update)
        const current = get()
        const navigates = (next.projectPagePath !== undefined && next.projectPagePath !== current.projectPagePath)
            || (current.viewMode === 'settings' && next.viewMode !== undefined && next.viewMode !== current.viewMode)
        if (navigates) requestNavigation(proceed)
        else proceed()
    }
    return ({
    viewMode: restoredRouteState.viewMode,
    setViewMode: (mode) => {
        const nextViewMode = resolveViewModeForProjectScope(mode)
        // Leaving an open chat remembers its project, so a flow run can default to it.
        set((state) => state.viewMode === 'home' && !state.projectPagePath && nextViewMode !== 'home'
            ? { viewMode: nextViewMode, chatOriginProjectPath: state.activeProjectPath }
            : { viewMode: nextViewMode })
    },
    chatOriginProjectPath: null,
    projectPagePath: restoredRouteState.projectPagePath,
    openProjectPage: (projectPath) => set({ projectPagePath: projectPath }),
    selectedMission: restoredRouteState.selectedMission,
    setSelectedMission: (selection) => set({ selectedMission: selection }),
    settingsCategory: restoredRouteState.settingsCategory,
    setSettingsCategory: (category) => set({ settingsCategory: category }),
    missionBoard: [],
    setMissionBoard: (update) => rawSet((state) => ({ missionBoard: update(state.missionBoard) })),
    activeProjectPath: restoredRouteState.activeProjectPath,
    projectRegistry: initialProjectRegistry,
    recentProjectPaths: restoredRouteState.activeProjectPath ? [restoredRouteState.activeProjectPath] : [],
    projectSessionsByPath: initialProjectSessionStates,
    hydrateProjectRegistry: (projects) =>
        set((state) => {
            const nextState = buildHydrateProjectRegistryTransition(state, projects)
            return nextState
        }),
    upsertProjectRegistryEntry: (project) =>
        set((state) => {
            const registeredProject = buildRegisteredProject(project)
            if (!registeredProject) {
                return state
            }
            const normalizedPath = registeredProject.directoryPath
            const nextProjectRegistry = {
                ...state.projectRegistry,
                [normalizedPath]: registeredProject,
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
            return nextState
        }),
    setActiveProjectPath: (projectPath) =>
        set((state) => {
            const nextState = buildSetActiveProjectTransition(state, projectPath)
            if (!nextState) {
                return state
            }
            // Choosing a chat's project shows the chat, not a project page.
            return { ...nextState, projectPagePath: null }
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
                ...state.homeConversationCache,
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

            result = {
                ok: true,
                normalizedPath: normalizedNextPath,
            }
            return {
                projectRegistry: nextProjectRegistry,
                projectSessionsByPath: nextProjectSessionStates,
                activeProjectPath: nextActiveProjectPath,
                projectPagePath: state.projectPagePath === normalizedCurrentPath ? normalizedNextPath : state.projectPagePath,
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
            // A project's draft belongs to its selected chat, so switching chats clears it.
            // Restoring a selection onto a project with none keeps text typed meanwhile.
            const homeSession = state.homeProjectSessionsByPath[normalizedProjectPath]
            const switchesChat = scoped.conversationId !== null && nextScopedWorkspace.conversationId !== scoped.conversationId
            const draftReset = homeSession && switchesChat
                ? {
                    homeProjectSessionsByPath: {
                        ...state.homeProjectSessionsByPath,
                        [normalizedProjectPath]: {
                            ...homeSession,
                            chatDraft: '',
                            pendingConversationTurn: null,
                        },
                    },
                }
                : {}
            const isActiveScope = state.activeProjectPath === normalizedProjectPath
            if (!isActiveScope) {
                return {
                    projectSessionsByPath: nextProjectSessionStates,
                    ...draftReset,
                }
            }
            return {
                projectSessionsByPath: nextProjectSessionStates,
                workingDir: nextScopedWorkspace.workingDir,
                ...draftReset,
            }
        }),
})
}
