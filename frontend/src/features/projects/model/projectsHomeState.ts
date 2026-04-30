import type {
    HydratedProjectRecord,
    ProjectRegistrationResult,
} from '@/store'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    ApiHttpError,
    type ConversationSegmentResponse,
    type ConversationSegmentUpsertEventResponse,
    type ConversationSnapshotResponse,
    type ConversationSummaryResponse,
    type ConversationTurnResponse,
    type ConversationTurnUpsertEventResponse,
    type FlowLaunchResponse,
    type FlowRunRequestResponse,
    type ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'
import type { ProjectGitMetadata } from './presentation'
import {
    ensureConversationSnapshotShell,
    sanitizeStreamingTurnUpsert,
    sortConversationSummaries,
    upsertConversationSummary,
} from './conversationState'
import { buildConversationTimelineEntriesForTurn } from './conversationTimeline'
import type { ConversationTimelineEntry } from './types'

export type ConversationStreamEvent = ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse

export type NormalizedConversationRecord = {
    schema_version: ConversationSnapshotResponse['schema_version']
    revision: ConversationSnapshotResponse['revision']
    conversation_id: string
    conversation_handle: string | null
    project_path: string
    chat_mode: ConversationSnapshotResponse['chat_mode']
    provider?: string | null
    model: string | null
    reasoning_effort: string | null
    title: string
    created_at: string
    updated_at: string
    orderedTurnIds: string[]
    turnsById: Record<string, ConversationTurnResponse>
    orderedSegmentIdsByTurnId: Record<string, string[]>
    segmentsById: Record<string, ConversationSegmentResponse>
    event_log: ConversationSnapshotResponse['event_log']
    flowRunRequestIds: string[]
    flowRunRequestsById: Record<string, FlowRunRequestResponse>
    flowLaunchIds: string[]
    flowLaunchesById: Record<string, FlowLaunchResponse>
    proposedPlanIds: string[]
    proposedPlansById: Record<string, ProposedPlanArtifactResponse>
    timelineEntryIds: string[]
    timelineEntriesById: Record<string, ConversationTimelineEntry>
    timelineEntryIdsByTurnId: Record<string, string[]>
}

export type ProjectConversationCacheState = {
    conversationsById: Record<string, NormalizedConversationRecord>
    summariesByProjectPath: Record<string, ConversationSummaryResponse[]>
}

export const EMPTY_PROJECT_CONVERSATION_CACHE_STATE: ProjectConversationCacheState = {
    conversationsById: {},
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

function indexById<T extends { id: string }>(items: T[]): Record<string, T> {
    return Object.fromEntries(items.map((item) => [item.id, item]))
}

function buildConversationSummaryFromRecord(record: NormalizedConversationRecord): ConversationSummaryResponse {
    const lastMessageTurn = record.orderedTurnIds
        .map((turnId) => record.turnsById[turnId])
        .filter((turn) => turn?.kind === 'message' && typeof turn.content === 'string' && turn.content.trim().length > 0)
        .slice(-1)[0]
    return {
        conversation_id: record.conversation_id,
        conversation_handle: record.conversation_handle,
        project_path: record.project_path,
        title: record.title,
        created_at: record.created_at,
        updated_at: record.updated_at,
        revision: record.revision,
        last_message_preview: lastMessageTurn?.content || null,
    }
}

export function getConversationTimelineEntries(record: NormalizedConversationRecord | null): ConversationTimelineEntry[] {
    if (!record) {
        return []
    }
    return record.timelineEntryIds
        .map((entryId) => record.timelineEntriesById[entryId])
        .filter((entry): entry is ConversationTimelineEntry => Boolean(entry))
}

export function getConversationFlowRunRequests(record: NormalizedConversationRecord | null): FlowRunRequestResponse[] {
    return record ? record.flowRunRequestIds.map((id) => record.flowRunRequestsById[id]).filter(Boolean) : []
}

export function getConversationFlowLaunches(record: NormalizedConversationRecord | null): FlowLaunchResponse[] {
    return record ? record.flowLaunchIds.map((id) => record.flowLaunchesById[id]).filter(Boolean) : []
}

export function getConversationProposedPlans(record: NormalizedConversationRecord | null): ProposedPlanArtifactResponse[] {
    return record ? record.proposedPlanIds.map((id) => record.proposedPlansById[id]).filter(Boolean) : []
}

function rebuildTurnTimelineEntries(
    record: NormalizedConversationRecord,
    turnId: string,
): NormalizedConversationRecord {
    const turn = record.turnsById[turnId]
    const previousTurnEntryIds = record.timelineEntryIdsByTurnId[turnId] || []
    const previousTurnEntryIdSet = new Set(previousTurnEntryIds)
    const nextTurnEntries = turn
        ? buildConversationTimelineEntriesForTurn(
            turn,
            (record.orderedSegmentIdsByTurnId[turnId] || [])
                .map((segmentId) => record.segmentsById[segmentId])
                .filter(Boolean),
        )
        : []
    const nextTurnEntryIds = nextTurnEntries.map((entry) => entry.id)
    const nextTimelineEntriesById = { ...record.timelineEntriesById }
    previousTurnEntryIds.forEach((entryId) => {
        delete nextTimelineEntriesById[entryId]
    })
    nextTurnEntries.forEach((entry) => {
        nextTimelineEntriesById[entry.id] = entry
    })

    let inserted = false
    const nextTimelineEntryIds = record.timelineEntryIds.flatMap((entryId) => {
        if (!previousTurnEntryIdSet.has(entryId)) {
            return [entryId]
        }
        if (inserted) {
            return []
        }
        inserted = true
        return nextTurnEntryIds
    })
    if (!inserted && nextTurnEntryIds.length > 0) {
        const turnIndex = record.orderedTurnIds.indexOf(turnId)
        let insertAt = nextTimelineEntryIds.length
        for (let index = turnIndex + 1; index < record.orderedTurnIds.length; index += 1) {
            const followingEntryId = record.timelineEntryIdsByTurnId[record.orderedTurnIds[index]]?.[0]
            if (followingEntryId) {
                const followingIndex = nextTimelineEntryIds.indexOf(followingEntryId)
                if (followingIndex >= 0) {
                    insertAt = followingIndex
                    break
                }
            }
        }
        nextTimelineEntryIds.splice(insertAt, 0, ...nextTurnEntryIds)
    }

    return {
        ...record,
        timelineEntryIds: nextTimelineEntryIds,
        timelineEntriesById: nextTimelineEntriesById,
        timelineEntryIdsByTurnId: {
            ...record.timelineEntryIdsByTurnId,
            [turnId]: nextTurnEntryIds,
        },
    }
}

export function hydrateConversationRecordFromSnapshot(
    snapshot: ConversationSnapshotResponse,
): NormalizedConversationRecord {
    const orderedSegmentIdsByTurnId: Record<string, string[]> = {}
    snapshot.segments.forEach((segment) => {
        orderedSegmentIdsByTurnId[segment.turn_id] = [
            ...(orderedSegmentIdsByTurnId[segment.turn_id] || []),
            segment.id,
        ]
    })
    Object.keys(orderedSegmentIdsByTurnId).forEach((turnId) => {
        orderedSegmentIdsByTurnId[turnId].sort((leftId, rightId) => {
            const left = snapshot.segments.find((segment) => segment.id === leftId)
            const right = snapshot.segments.find((segment) => segment.id === rightId)
            if (!left || !right) {
                return leftId.localeCompare(rightId)
            }
            const orderDelta = left.order - right.order
            if (orderDelta !== 0) {
                return orderDelta
            }
            const timestampDelta = left.timestamp.localeCompare(right.timestamp)
            return timestampDelta !== 0 ? timestampDelta : left.id.localeCompare(right.id)
        })
    })

    let record: NormalizedConversationRecord = {
        schema_version: snapshot.schema_version,
        revision: snapshot.revision,
        conversation_id: snapshot.conversation_id,
        conversation_handle: snapshot.conversation_handle ?? null,
        project_path: snapshot.project_path,
        chat_mode: snapshot.chat_mode,
        provider: snapshot.provider ?? null,
        model: snapshot.model ?? null,
        reasoning_effort: snapshot.reasoning_effort ?? null,
        title: snapshot.title,
        created_at: snapshot.created_at,
        updated_at: snapshot.updated_at,
        orderedTurnIds: snapshot.turns.map((turn) => turn.id),
        turnsById: indexById(snapshot.turns),
        orderedSegmentIdsByTurnId,
        segmentsById: indexById(snapshot.segments),
        event_log: snapshot.event_log,
        flowRunRequestIds: snapshot.flow_run_requests.map((request) => request.id),
        flowRunRequestsById: indexById(snapshot.flow_run_requests),
        flowLaunchIds: snapshot.flow_launches.map((launch) => launch.id),
        flowLaunchesById: indexById(snapshot.flow_launches),
        proposedPlanIds: (snapshot.proposed_plans || []).map((plan) => plan.id),
        proposedPlansById: indexById(snapshot.proposed_plans || []),
        timelineEntryIds: [],
        timelineEntriesById: {},
        timelineEntryIdsByTurnId: {},
    }
    record.orderedTurnIds.forEach((turnId) => {
        record = rebuildTurnTimelineEntries(record, turnId)
    })
    return record
}

function isConversationRecordAtLeastAsFreshAsSnapshot(
    record: NormalizedConversationRecord,
    snapshot: ConversationSnapshotResponse,
): boolean {
    return record.revision >= snapshot.revision
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
    const existingRecord = current.conversationsById[scopedSnapshot.conversation_id]
    if (existingRecord && isConversationRecordAtLeastAsFreshAsSnapshot(existingRecord, scopedSnapshot)) {
        return {
            applied: false,
            cache: current,
        }
    }
    const record = hydrateConversationRecordFromSnapshot(scopedSnapshot)

    return {
        applied: true,
        record,
        cache: {
            conversationsById: {
                ...current.conversationsById,
                [scopedSnapshot.conversation_id]: record,
            },
            summariesByProjectPath: {
                ...current.summariesByProjectPath,
                [projectPath]: upsertConversationSummary(
                    current.summariesByProjectPath[projectPath] || [],
                    buildConversationSummaryFromRecord(record),
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
    const cachedRecord = current.conversationsById[event.conversation_id]
    if (cachedRecord && event.revision < cachedRecord.revision) {
        return {
            record: cachedRecord,
            cache: current,
        }
    }
    const existingRecord = cachedRecord
        || hydrateConversationRecordFromSnapshot(ensureConversationSnapshotShell(event.conversation_id, projectPath, event.title))
    let mergedRecord: NormalizedConversationRecord = {
        ...existingRecord,
        project_path: projectPath,
        title: event.title,
        updated_at: event.updated_at,
        revision: event.revision,
    }
    if (event.type === 'turn_upsert') {
        const currentTurn = existingRecord.turnsById[event.turn.id] || null
        const turn = sanitizeStreamingTurnUpsert(currentTurn, event.turn)
        mergedRecord = {
            ...mergedRecord,
            chat_mode: event.turn.kind === 'mode_change'
                ? (event.turn.content === 'plan' ? 'plan' : 'chat')
                : existingRecord.chat_mode,
            orderedTurnIds: existingRecord.turnsById[turn.id]
                ? existingRecord.orderedTurnIds
                : [...existingRecord.orderedTurnIds, turn.id],
            turnsById: {
                ...existingRecord.turnsById,
                [turn.id]: turn,
            },
        }
        mergedRecord = rebuildTurnTimelineEntries(mergedRecord, turn.id)
    } else if (event.type === 'segment_upsert') {
        const segment = event.segment
        const turnSegmentIds = existingRecord.orderedSegmentIdsByTurnId[segment.turn_id] || []
        const nextTurnSegmentIds = turnSegmentIds.includes(segment.id)
            ? turnSegmentIds
            : [...turnSegmentIds, segment.id]
        const nextSegmentsById = {
            ...existingRecord.segmentsById,
            [segment.id]: segment,
        }
        nextTurnSegmentIds.sort((leftId, rightId) => {
            const left = nextSegmentsById[leftId]
            const right = nextSegmentsById[rightId]
            if (!left || !right) {
                return leftId.localeCompare(rightId)
            }
            const orderDelta = left.order - right.order
            if (orderDelta !== 0) {
                return orderDelta
            }
            return left.timestamp.localeCompare(right.timestamp) || left.id.localeCompare(right.id)
        })
        mergedRecord = {
            ...mergedRecord,
            segmentsById: nextSegmentsById,
            orderedSegmentIdsByTurnId: {
                ...existingRecord.orderedSegmentIdsByTurnId,
                [segment.turn_id]: nextTurnSegmentIds,
            },
        }
        mergedRecord = rebuildTurnTimelineEntries(mergedRecord, segment.turn_id)
    }

    return {
        record: mergedRecord,
        cache: {
            conversationsById: {
                ...current.conversationsById,
                [event.conversation_id]: mergedRecord,
            },
            summariesByProjectPath: {
                ...current.summariesByProjectPath,
                [projectPath]: upsertConversationSummary(
                    current.summariesByProjectPath[projectPath] || [],
                    buildConversationSummaryFromRecord(mergedRecord),
                ),
            },
        },
    }
}

export function removeConversationFromCache(
    current: ProjectConversationCacheState,
    conversationId: string,
): ProjectConversationCacheState {
    const nextConversations = { ...current.conversationsById }
    delete nextConversations[conversationId]
    return {
        ...current,
        conversationsById: nextConversations,
    }
}

export function removeProjectFromCache(
    current: ProjectConversationCacheState,
    projectPath: string,
): ProjectConversationCacheState {
    const nextSummariesByProjectPath = { ...current.summariesByProjectPath }
    delete nextSummariesByProjectPath[projectPath]

    const nextConversationsById: Record<string, NormalizedConversationRecord> = {}
    Object.entries(current.conversationsById).forEach(([conversationId, conversation]) => {
        if (conversation.project_path !== projectPath) {
            nextConversationsById[conversationId] = conversation
        }
    })

    return {
        conversationsById: nextConversationsById,
        summariesByProjectPath: nextSummariesByProjectPath,
    }
}
