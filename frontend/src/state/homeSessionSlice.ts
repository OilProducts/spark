import { type StateCreator } from 'zustand'
import type { ProjectGitMetadata } from '@/features/projects/model/presentation'
import type { AppState } from './store-types'
import type {
    HomeConversationSessionState,
    HomeProjectSessionState,
    HomeSessionSlice,
} from './viewSessionTypes'

const DEFAULT_HOME_PROJECT_SESSION_STATE: HomeProjectSessionState = {
    chatDraft: '',
    panelError: null,
    pendingConversationTurn: null,
    pendingDeleteConversationId: null,
}

const DEFAULT_HOME_CONVERSATION_SESSION_STATE: HomeConversationSessionState = {
    expandedToolCalls: {},
    expandedThinkingEntries: {},
    isPinnedToBottom: true,
    scrollTop: null,
    attachments: [],
    pendingMessages: [],
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

// The chats' fresh starts. A switch away and back starts them anew in the same environment, so a
// request begun before then is no longer the shown chats', though its environment is shown.
let homeSessionStarts = 0
/** The chats' current fresh start. */
export const homeSessionStart = () => homeSessionStarts

// Set while the app opens a project's chat by itself, as its latest once its list loads: no one chose it.
let openingByItself = false
/** Whether the chat opening now is one the app opens by itself. */
export const opensByItself = () => openingByItself
/** Opens a chat through `open` as the app's own choice, not yours. */
export function openByItself(open: () => void) {
    openingByItself = true
    try {
        open()
    } finally {
        openingByItself = false
    }
}

/** The chats' state in a fresh environment: all of it belongs to one environment. */
export const initialHomeSessionState = () => {
    homeSessionStarts += 1
    return {
        homeConversationCache: {
            conversationsById: {},
            summariesByProjectPath: {},
        },
        homeThreadSummariesStatusByProjectPath: {},
        homeThreadSummariesErrorByProjectPath: {},
        homeProjectSessionsByPath: {},
        homeConversationSessionsById: {},
        homeProjectGitMetadataByPath: {},
        sendingChats: {},
    } satisfies Partial<HomeSessionSlice>
}

export const createHomeSessionSlice: StateCreator<AppState, [], [], HomeSessionSlice> = (set) => ({
    ...initialHomeSessionState(),
    setSendingChat: (conversationId, sending) =>
        set((state) => {
            const sendingChats = { ...state.sendingChats }
            if (sending) sendingChats[conversationId] = true
            else delete sendingChats[conversationId]
            return { sendingChats }
        }),
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

            const nextConversationsById = { ...state.homeConversationCache.conversationsById }
            const nextConversationSessionsById = { ...state.homeConversationSessionsById }
            Object.entries(state.homeConversationCache.conversationsById).forEach(([conversationId, conversation]) => {
                if (conversation.project_path === projectPath) {
                    removedConversationIds.add(conversationId)
                    delete nextConversationsById[conversationId]
                }
            })
            removedConversationIds.forEach((conversationId) => {
                delete nextConversationSessionsById[conversationId]
            })

            return {
                homeConversationCache: {
                    ...state.homeConversationCache,
                    conversationsById: nextConversationsById,
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

            const nextConversationsById = Object.fromEntries(
                Object.entries(state.homeConversationCache.conversationsById).map(([conversationId, conversation]) => ([
                    conversationId,
                    conversation.project_path === currentProjectPath
                        ? {
                            ...conversation,
                            project_path: nextProjectPath,
                        }
                        : conversation,
                ])),
            )

            return {
                homeConversationCache: {
                    ...state.homeConversationCache,
                    conversationsById: nextConversationsById,
                    summariesByProjectPath: nextSummariesByProjectPath,
                },
                homeThreadSummariesStatusByProjectPath: nextThreadStatuses,
                homeThreadSummariesErrorByProjectPath: nextThreadErrors,
                homeProjectSessionsByPath: nextProjectSessions,
                homeProjectGitMetadataByPath: nextGitMetadata,
            }
        }),
})
