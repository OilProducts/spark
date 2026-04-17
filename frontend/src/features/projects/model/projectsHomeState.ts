import type {
    HydratedProjectRecord,
    ProjectRegistrationResult,
} from '@/store'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    ApiHttpError,
    type ConversationSegmentUpsertEventResponse,
    type ConversationSnapshotResponse,
    type ConversationSummaryResponse,
    type ConversationTurnUpsertEventResponse,
} from '@/lib/workspaceClient'
import type { ProjectGitMetadata } from './presentation'
import {
    compareConversationSnapshotFreshness,
    ensureConversationSnapshotShell,
    sanitizeStreamingTurnUpsert,
    sortConversationSummaries,
    upsertConversationSegment,
    upsertConversationSummary,
    upsertConversationTurn,
} from './conversationState'

export type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

export type ProjectConversationCacheState = {
    snapshotsByConversationId: Record<string, ConversationSnapshotResponse>
    summariesByProjectPath: Record<string, ConversationSummaryResponse[]>
}

export const EMPTY_PROJECT_CONVERSATION_CACHE_STATE: ProjectConversationCacheState = {
    snapshotsByConversationId: {},
    summariesByProjectPath: {},
}

export const EMPTY_PROJECT_GIT_METADATA: ProjectGitMetadata = {
    branch: null,
    commit: null,
}

export function buildProjectConversationId(projectPath: string) {
    const normalizedProjectKey = projectPath
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/(^-|-$)/g, '')
    const suffix = normalizedProjectKey || 'project'
    const randomSuffix = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
        ? crypto.randomUUID().slice(0, 8)
        : Math.random().toString(36).slice(2, 10)
    return `conversation-${suffix}-${randomSuffix}`
}

export function asProjectGitMetadataField(value: unknown): string | null {
    if (typeof value !== 'string') {
        return null
    }
    const trimmed = value.trim()
    return trimmed.length > 0 ? trimmed : null
}

export function formatProjectListLabel(projectPath: string) {
    const normalizedPath = normalizeProjectPath(projectPath)
    const segments = normalizedPath.split('/').filter(Boolean)
    if (segments.length === 0) {
        return normalizedPath
    }
    return segments[segments.length - 1]
}

export function toHydratedProjectRecord(project: {
    project_path: string
    is_favorite: boolean
    last_accessed_at?: string | null
    active_conversation_id?: string | null
}): HydratedProjectRecord {
    return {
        directoryPath: project.project_path,
        isFavorite: project.is_favorite === true,
        lastAccessedAt: typeof project.last_accessed_at === 'string' ? project.last_accessed_at : null,
        activeConversationId: typeof project.active_conversation_id === 'string' ? project.active_conversation_id : null,
    }
}

export function formatConversationAgeShort(value: string) {
    const parsed = new Date(value)
    if (Number.isNaN(parsed.getTime())) {
        return ''
    }
    const deltaMs = Date.now() - parsed.getTime()
    if (deltaMs <= 0) {
        return 'now'
    }
    const minuteMs = 60_000
    const hourMs = 60 * minuteMs
    const dayMs = 24 * hourMs
    const weekMs = 7 * dayMs
    if (deltaMs < hourMs) {
        return `${Math.max(1, Math.round(deltaMs / minuteMs))}m`
    }
    if (deltaMs < dayMs) {
        return `${Math.max(1, Math.round(deltaMs / hourMs))}h`
    }
    if (deltaMs < weekMs) {
        return `${Math.max(1, Math.round(deltaMs / dayMs))}d`
    }
    return `${Math.max(1, Math.round(deltaMs / weekMs))}w`
}

export function formatConversationTimestamp(value: string) {
    const parsed = new Date(value)
    if (Number.isNaN(parsed.getTime())) {
        return value
    }
    return parsed.toLocaleString()
}

export function extractApiErrorMessage(error: unknown, fallback: string) {
    if (error instanceof ApiHttpError && error.detail) {
        return error.detail
    }
    if (error instanceof Error && error.message) {
        return error.message
    }
    return fallback
}

export function buildOrderedProjects<ProjectRecord extends { directoryPath: string }>(
    projects: ProjectRecord[],
    projectRegistry: Record<string, ProjectRecord>,
    recentProjectPaths: string[],
) {
    const seenProjectPaths = new Set<string>()
    const items: ProjectRecord[] = []

    recentProjectPaths.forEach((projectPath) => {
        const project = projectRegistry[projectPath]
        if (!project || seenProjectPaths.has(projectPath)) {
            return
        }
        items.push(project)
        seenProjectPaths.add(projectPath)
    })

    projects.forEach((project) => {
        if (seenProjectPaths.has(project.directoryPath)) {
            return
        }
        items.push(project)
        seenProjectPaths.add(project.directoryPath)
    })

    return items
}

export function resolveProjectPathValidation(
    rawPath: string,
    projectRegistry: Record<string, unknown>,
): ProjectRegistrationResult {
    const normalizedPath = normalizeProjectPath(rawPath)
    if (!normalizedPath) {
        return { ok: false, error: 'Project directory path is required.' }
    }
    if (!isAbsoluteProjectPath(normalizedPath)) {
        return {
            ok: false,
            normalizedPath,
            error: 'Project directory path must be absolute.',
        }
    }
    const duplicate = Boolean(projectRegistry[normalizedPath])
    if (duplicate) {
        return {
            ok: false,
            normalizedPath,
            error: `Project already registered: ${normalizedPath}`,
        }
    }
    return {
        ok: true,
        normalizedPath,
    }
}

function buildConversationSummaryFromSnapshot(snapshot: ConversationSnapshotResponse): ConversationSummaryResponse {
    return {
        conversation_id: snapshot.conversation_id,
        conversation_handle: snapshot.conversation_handle,
        project_path: snapshot.project_path,
        title: snapshot.title,
        created_at: snapshot.created_at,
        updated_at: snapshot.updated_at,
        last_message_preview: snapshot.turns
            .filter((turn) => turn.kind === 'message' && typeof turn.content === 'string' && turn.content.trim().length > 0)
            .slice(-1)[0]?.content || null,
    }
}

export function setProjectConversationSummaryList(
    current: ProjectConversationCacheState,
    projectPath: string,
    summaries: ConversationSummaryResponse[],
): ProjectConversationCacheState {
    return {
        ...current,
        summariesByProjectPath: {
            ...current.summariesByProjectPath,
            [projectPath]: sortConversationSummaries(summaries),
        },
    }
}

export function applyConversationSnapshotToCache(
    current: ProjectConversationCacheState,
    projectPath: string,
    snapshot: ConversationSnapshotResponse,
) {
    const scopedSnapshot = snapshot.project_path === projectPath
        ? snapshot
        : {
            ...snapshot,
            project_path: projectPath,
        }
    const existingSnapshot = current.snapshotsByConversationId[scopedSnapshot.conversation_id]
    if (existingSnapshot && compareConversationSnapshotFreshness(existingSnapshot, scopedSnapshot) >= 0) {
        return {
            applied: false,
            cache: current,
        }
    }

    return {
        applied: true,
        cache: {
            snapshotsByConversationId: {
                ...current.snapshotsByConversationId,
                [scopedSnapshot.conversation_id]: scopedSnapshot,
            },
            summariesByProjectPath: {
                ...current.summariesByProjectPath,
                [projectPath]: upsertConversationSummary(
                    current.summariesByProjectPath[projectPath] || [],
                    buildConversationSummaryFromSnapshot(scopedSnapshot),
                ),
            },
        },
    }
}

export function applyConversationStreamEventToCache(
    current: ProjectConversationCacheState,
    projectPath: string,
    event: ConversationStreamEvent,
) {
    const existingSnapshot = current.snapshotsByConversationId[event.conversation_id]
        || ensureConversationSnapshotShell(event.conversation_id, projectPath, event.title)
    let mergedSnapshot = existingSnapshot
    if (event.type === 'turn_upsert') {
        const currentTurn = existingSnapshot.turns.find((turn) => turn.id === event.turn.id) || null
        mergedSnapshot = {
            ...upsertConversationTurn(existingSnapshot, sanitizeStreamingTurnUpsert(currentTurn, event.turn)),
            chat_mode: event.turn.kind === 'mode_change'
                ? (event.turn.content === 'plan' ? 'plan' : 'chat')
                : existingSnapshot.chat_mode,
            project_path: projectPath,
            title: event.title,
            updated_at: event.updated_at,
        }
    } else {
        mergedSnapshot = {
            ...existingSnapshot,
            project_path: projectPath,
            title: event.title,
            updated_at: event.updated_at,
        }
    }
    if (event.type === 'segment_upsert') {
        mergedSnapshot = upsertConversationSegment(mergedSnapshot, event.segment)
    }

    return {
        snapshot: mergedSnapshot,
        cache: {
            snapshotsByConversationId: {
                ...current.snapshotsByConversationId,
                [event.conversation_id]: mergedSnapshot,
            },
            summariesByProjectPath: {
                ...current.summariesByProjectPath,
                [projectPath]: upsertConversationSummary(
                    current.summariesByProjectPath[projectPath] || [],
                    buildConversationSummaryFromSnapshot(mergedSnapshot),
                ),
            },
        },
    }
}

export function removeConversationFromCache(
    current: ProjectConversationCacheState,
    conversationId: string,
): ProjectConversationCacheState {
    const nextSnapshots = { ...current.snapshotsByConversationId }
    delete nextSnapshots[conversationId]
    return {
        ...current,
        snapshotsByConversationId: nextSnapshots,
    }
}

export function removeProjectFromCache(
    current: ProjectConversationCacheState,
    projectPath: string,
): ProjectConversationCacheState {
    const nextSummariesByProjectPath = { ...current.summariesByProjectPath }
    delete nextSummariesByProjectPath[projectPath]

    const nextSnapshotsByConversationId: Record<string, ConversationSnapshotResponse> = {}
    Object.entries(current.snapshotsByConversationId).forEach(([conversationId, snapshot]) => {
        if (snapshot.project_path !== projectPath) {
            nextSnapshotsByConversationId[conversationId] = snapshot
        }
    })

    return {
        snapshotsByConversationId: nextSnapshotsByConversationId,
        summariesByProjectPath: nextSummariesByProjectPath,
    }
}
