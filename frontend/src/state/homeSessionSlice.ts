import { type StateCreator } from 'zustand'
import type { ProjectGitMetadata } from '@/features/projects/model/presentation'
import { sortConversationSummaries } from '@/features/projects/model/conversationState'
import type { AppState } from './store-types'
import type {
    HomeConversationSessionState,
    HomeProjectSessionState,
    HomeSessionSlice,
} from './viewSessionTypes'

const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320

const DEFAULT_HOME_PROJECT_SESSION_STATE: HomeProjectSessionState = {
    chatDraft: '',
    panelError: null,
    optimisticSend: null,
    pendingDeleteConversationId: null,
    sidebarPrimaryHeight: DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT,
}

const DEFAULT_HOME_CONVERSATION_SESSION_STATE: HomeConversationSessionState = {
    expandedProposalChanges: {},
    expandedToolCalls: {},
    expandedThinkingEntries: {},
    isPinnedToBottom: true,
    scrollTop: null,
}

const EMPTY_GIT_METADATA: ProjectGitMetadata = {
    branch: null,
    commit: null,
}

const resolveHomeProjectSession = (
    sessionsByPath: Record<string, HomeProjectSessionState>,
    projectPath: string,
) => ({
    ...DEFAULT_HOME_PROJECT_SESSION_STATE,
    ...(sessionsByPath[projectPath] ?? {}),
})

const resolveHomeConversationSession = (
    sessionsById: Record<string, HomeConversationSessionState>,
    conversationId: string,
) => ({
    ...DEFAULT_HOME_CONVERSATION_SESSION_STATE,
    ...(sessionsById[conversationId] ?? {}),
})

export const createHomeSessionSlice: StateCreator<AppState, [], [], HomeSessionSlice> = (set) => ({
    homeConversationCache: {
        snapshotsByConversationId: {},
        summariesByProjectPath: {},
    },
    homeThreadSummariesStatusByProjectPath: {},
    homeThreadSummariesErrorByProjectPath: {},
    homeProjectSessionsByPath: {},
    homeConversationSessionsById: {},
    homeProjectGitMetadataByPath: {},
    updateHomeProjectSession: (projectPath, patch) =>
        set((state) => ({
            homeProjectSessionsByPath: {
                ...state.homeProjectSessionsByPath,
                [projectPath]: {
                    ...resolveHomeProjectSession(state.homeProjectSessionsByPath, projectPath),
                    ...patch,
                },
            },
        })),
    updateHomeConversationSession: (conversationId, patch) =>
        set((state) => ({
            homeConversationSessionsById: {
                ...state.homeConversationSessionsById,
                [conversationId]: {
                    ...resolveHomeConversationSession(state.homeConversationSessionsById, conversationId),
                    ...patch,
                },
            },
        })),
    commitHomeConversationCache: (next) =>
        set((state) => ({
            homeConversationCache: typeof next === 'function' ? next(state.homeConversationCache) : next,
        })),
    setHomeConversationSummaryList: (projectPath, summaries) =>
        set((state) => ({
            homeConversationCache: {
                ...state.homeConversationCache,
                summariesByProjectPath: {
                    ...state.homeConversationCache.summariesByProjectPath,
                    [projectPath]: sortConversationSummaries(summaries),
                },
            },
        })),
    setHomeThreadSummariesStatus: (projectPath, status, error = null) =>
        set((state) => ({
            homeThreadSummariesStatusByProjectPath: {
                ...state.homeThreadSummariesStatusByProjectPath,
                [projectPath]: status,
            },
            homeThreadSummariesErrorByProjectPath: {
                ...state.homeThreadSummariesErrorByProjectPath,
                [projectPath]: error,
            },
        })),
    setHomeProjectGitMetadata: (projectPath, metadata) =>
        set((state) => {
            const current = state.homeProjectGitMetadataByPath[projectPath] ?? EMPTY_GIT_METADATA
            return {
                homeProjectGitMetadataByPath: {
                    ...state.homeProjectGitMetadataByPath,
                    [projectPath]: typeof metadata === 'function' ? metadata(current) : metadata,
                },
            }
        }),
    clearHomeConversationSession: (conversationId) =>
        set((state) => {
            if (!(conversationId in state.homeConversationSessionsById)) {
                return state
            }
            const nextConversationSessionsById = { ...state.homeConversationSessionsById }
            delete nextConversationSessionsById[conversationId]
            return {
                homeConversationSessionsById: nextConversationSessionsById,
            }
        }),
    removeHomeProjectSession: (projectPath) =>
        set((state) => {
            const nextProjectSessions = { ...state.homeProjectSessionsByPath }
            delete nextProjectSessions[projectPath]

            const nextThreadStatuses = { ...state.homeThreadSummariesStatusByProjectPath }
            delete nextThreadStatuses[projectPath]

            const nextThreadErrors = { ...state.homeThreadSummariesErrorByProjectPath }
            delete nextThreadErrors[projectPath]

            const nextGitMetadata = { ...state.homeProjectGitMetadataByPath }
            delete nextGitMetadata[projectPath]

            const nextSummariesByProjectPath = { ...state.homeConversationCache.summariesByProjectPath }
            const removedConversationIds = new Set(
                (nextSummariesByProjectPath[projectPath] ?? []).map((summary) => summary.conversation_id),
            )
            delete nextSummariesByProjectPath[projectPath]

            const nextSnapshotsByConversationId = { ...state.homeConversationCache.snapshotsByConversationId }
            const nextConversationSessionsById = { ...state.homeConversationSessionsById }
            Object.entries(state.homeConversationCache.snapshotsByConversationId).forEach(([conversationId, snapshot]) => {
                if (snapshot.project_path === projectPath) {
                    removedConversationIds.add(conversationId)
                    delete nextSnapshotsByConversationId[conversationId]
                }
            })
            removedConversationIds.forEach((conversationId) => {
                delete nextConversationSessionsById[conversationId]
            })

            return {
                homeConversationCache: {
                    snapshotsByConversationId: nextSnapshotsByConversationId,
                    summariesByProjectPath: nextSummariesByProjectPath,
                },
                homeThreadSummariesStatusByProjectPath: nextThreadStatuses,
                homeThreadSummariesErrorByProjectPath: nextThreadErrors,
                homeProjectSessionsByPath: nextProjectSessions,
                homeConversationSessionsById: nextConversationSessionsById,
                homeProjectGitMetadataByPath: nextGitMetadata,
            }
        }),
    renameHomeProjectSession: (currentProjectPath, nextProjectPath) =>
        set((state) => {
            if (currentProjectPath === nextProjectPath) {
                return state
            }

            const nextProjectSessions = { ...state.homeProjectSessionsByPath }
            if (nextProjectSessions[currentProjectPath]) {
                nextProjectSessions[nextProjectPath] = resolveHomeProjectSession(
                    nextProjectSessions,
                    currentProjectPath,
                )
                delete nextProjectSessions[currentProjectPath]
            }

            const nextThreadStatuses = { ...state.homeThreadSummariesStatusByProjectPath }
            if (currentProjectPath in nextThreadStatuses) {
                nextThreadStatuses[nextProjectPath] = nextThreadStatuses[currentProjectPath]
                delete nextThreadStatuses[currentProjectPath]
            }

            const nextThreadErrors = { ...state.homeThreadSummariesErrorByProjectPath }
            if (currentProjectPath in nextThreadErrors) {
                nextThreadErrors[nextProjectPath] = nextThreadErrors[currentProjectPath]
                delete nextThreadErrors[currentProjectPath]
            }

            const nextGitMetadata = { ...state.homeProjectGitMetadataByPath }
            if (currentProjectPath in nextGitMetadata) {
                nextGitMetadata[nextProjectPath] = nextGitMetadata[currentProjectPath]
                delete nextGitMetadata[currentProjectPath]
            }

            const nextSummariesByProjectPath = { ...state.homeConversationCache.summariesByProjectPath }
            if (currentProjectPath in nextSummariesByProjectPath) {
                nextSummariesByProjectPath[nextProjectPath] = nextSummariesByProjectPath[currentProjectPath].map((summary) => ({
                    ...summary,
                    project_path: nextProjectPath,
                }))
                delete nextSummariesByProjectPath[currentProjectPath]
            }

            const nextSnapshotsByConversationId = Object.fromEntries(
                Object.entries(state.homeConversationCache.snapshotsByConversationId).map(([conversationId, snapshot]) => ([
                    conversationId,
                    snapshot.project_path === currentProjectPath
                        ? {
                            ...snapshot,
                            project_path: nextProjectPath,
                        }
                        : snapshot,
                ])),
            )

            return {
                homeConversationCache: {
                    snapshotsByConversationId: nextSnapshotsByConversationId,
                    summariesByProjectPath: nextSummariesByProjectPath,
                },
                homeThreadSummariesStatusByProjectPath: nextThreadStatuses,
                homeThreadSummariesErrorByProjectPath: nextThreadErrors,
                homeProjectSessionsByPath: nextProjectSessions,
                homeProjectGitMetadataByPath: nextGitMetadata,
            }
        }),
})
