import type {
    HydratedProjectRecord,
    ProjectRegistrationResult,
} from '@/store'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'
import {
    ApiHttpError,
    compareRevisions,
    conversationSummaryFromRow,
    type ConversationBatch,
    type ConversationPage,
    type ConversationRow,
    type ConversationRowPayload,
    type ConversationSegmentResponse,
    type ConversationStreamDeltaEventResponse,
    type ConversationSummaryResponse,
    type ConversationSummarySnapshot,
    type ConversationTurnResponse,
    type FlowLaunchResponse,
    type FlowRunRequestResponse,
    type ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'
import type { ProjectGitMetadata } from './presentation'
import {
    conversationRowKey,
    mergeConversationRows,
    sanitizeStreamingTurnUpsert,
    upsertConversationSummary,
    type ConversationRows,
} from './conversationState'
import { buildConversationTimelineEntriesForTurn } from './conversationTimeline'
import type { ConversationTimelineEntry } from './types'

export type { ConversationSummaryResponse } from '@/lib/workspaceClient'

export type ConversationStreamDelta = ConversationStreamDeltaEventResponse

/** What a conversation's loaded rows hold: its live cursor, loaded pages and stream overlay. */
type ConversationWindowState = {
    generation: string
    /**
     * The project path the client opened the conversation under. The server may spell a
     * project's path differently (its normalized form); the client keeps its own.
     */
    projectPath: string | null
    /** Live catch-up cursor, advanced only by windows and live batches; null before either. */
    watermark: string | null
    /** A live reset is pending: live batches are ignored until the next window. */
    resetting: boolean
    olderCursor: string | null
    /** The oldest loaded page's first turn position; older turns that a delta brings stay hidden. */
    windowStart: string | null
    pinnedTurnIds: string[]
    rows: ConversationRows
    /** Uncommitted stream content shown over committed rows, until the row commits. */
    streamTurnsById: Record<string, ConversationTurnResponse>
    streamSegmentsById: Record<string, ConversationSegmentResponse>
}

/** The normalized rows and the transcript view derived from them. */
export type NormalizedConversationRecord = ConversationWindowState & {
    /** The conversation row's revision. */
    revision: string
    /** The last committed settings change's revision: what a settings write expects. */
    settingsRevision: string
    /** The newest applied row revision. */
    latestRevision: string
    deleted: boolean
    model_settings_view?: ConversationRowPayload['model_settings_view']
    conversation_id: string
    conversation_handle: string | null
    project_path: string
    chat_mode: ConversationRowPayload['chat_mode']
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
    /** Deleted conversations' tombstone revisions, so an older summary cannot bring one back. */
    summaryTombstones?: Record<string, string>
    /** The all-projects summary feed's catch-up cursor. */
    summaryCursor?: { generation: string; watermark: string } | null
    /** The store generation the summary rows belong to; kept when a reset clears the cursor. */
    summaryGeneration?: string | null
    /** The newest live all-projects snapshot: an HTTP list read before it, or in another
     * generation, predates its membership and is ignored. */
    summaryAuthority?: { generation: string; watermark: string } | null
    /** Conversations that were deleted or are gone after a reset. */
    unavailableConversationIds?: Record<string, true>
    // Last applied live update's stream_sequence, by turn id.
    streamSequenceByTurnId?: Record<string, number>
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
    display_name?: string
    is_default?: boolean
    folder_exists?: boolean
    is_favorite: boolean
    last_accessed_at?: string | null
    active_conversation_id?: string | null
    execution_profile_id?: string | null
}): HydratedProjectRecord {
    return {
        directoryPath: project.project_path,
        ...(project.display_name ? { displayName: project.display_name } : {}),
        ...(project.is_default ? { isDefault: true } : {}),
        ...(project.folder_exists === false ? { folderExists: false } : {}),
        isFavorite: project.is_favorite === true,
        lastAccessedAt: typeof project.last_accessed_at === 'string' ? project.last_accessed_at : null,
        activeConversationId: typeof project.active_conversation_id === 'string' ? project.active_conversation_id : null,
        ...(typeof project.execution_profile_id === 'string'
            ? { executionProfileId: project.execution_profile_id }
            : {}),
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

type RowOf<K extends ConversationRow['kind']> = Extract<ConversationRow, { kind: K }>

/** Orders optional decimal strings; a missing one sorts last. */
function compareOptionalDecimals(left: string | undefined, right: string | undefined): number {
    if (left === undefined || right === undefined) {
        return left === right ? 0 : left === undefined ? 1 : -1
    }
    return compareRevisions(left, right)
}

const turnKey = (conversationId: string, id: string) => conversationRowKey({ kind: 'turn', conversation_id: conversationId, id })
const segmentKey = (conversationId: string, id: string) => conversationRowKey({ kind: 'segment', conversation_id: conversationId, id })

function emptyWindow(generation: string, projectPath: string | null): ConversationWindowState {
    return {
        generation,
        projectPath,
        watermark: null,
        resetting: false,
        olderCursor: null,
        windowStart: null,
        pinnedTurnIds: [],
        rows: {},
        streamTurnsById: {},
        streamSegmentsById: {},
    }
}

function windowOf(record: NormalizedConversationRecord): ConversationWindowState {
    const { generation, projectPath, watermark, resetting, olderCursor, windowStart, pinnedTurnIds, rows, streamTurnsById, streamSegmentsById } = record
    return { generation, projectPath, watermark, resetting, olderCursor, windowStart, pinnedTurnIds, rows, streamTurnsById, streamSegmentsById }
}

/** A deleted conversation keeps only its tombstone: every child it had is invalid. */
function dropChildrenOfDeletedConversation(conversationId: string, state: ConversationWindowState): ConversationWindowState {
    const key = conversationRowKey({ kind: 'conversation', conversation_id: conversationId, id: conversationId })
    const conversation = state.rows[key]
    if (!conversation?.deleted || Object.keys(state.rows).length === 1) {
        return state
    }
    return { ...state, rows: { [key]: conversation }, streamTurnsById: {}, streamSegmentsById: {} }
}

function sameTurnSegments(previous: NormalizedConversationRecord, turnId: string, segments: ConversationSegmentResponse[]) {
    const previousIds = previous.orderedSegmentIdsByTurnId[turnId] ?? []
    return previousIds.length === segments.length
        && segments.every((segment, index) => previous.segmentsById[previousIds[index]] === segment)
}

/**
 * The visible transcript from normalized rows: turns by position, segments by turn then order,
 * artifacts by collection then position, and each proposed plan linked to its source segment.
 * A turn whose turn and segments are unchanged reuses `previous`'s timeline entries.
 */
function deriveConversationRecord(
    conversationId: string,
    state: ConversationWindowState,
    previous?: NormalizedConversationRecord,
): NormalizedConversationRecord {
    let conversation: RowOf<'conversation'> | undefined
    const turnRows: RowOf<'turn'>[] = []
    const segmentRows: RowOf<'segment'>[] = []
    const artifactRows: Partial<Record<string, RowOf<'artifact'>[]>> = {}
    let latestRevision = '0'
    for (const row of Object.values(state.rows)) {
        if (compareRevisions(row.revision, latestRevision) > 0) latestRevision = row.revision
        if (row.kind === 'conversation') conversation = row
        if (row.deleted) continue
        if (row.kind === 'turn') turnRows.push(row)
        else if (row.kind === 'segment') segmentRows.push(row)
        else if (row.kind === 'artifact') (artifactRows[row.collection] ??= []).push(row)
    }
    const artifacts = <T>(collection: string): T[] => (artifactRows[collection] ?? [])
        .sort((left, right) => compareOptionalDecimals(left.position, right.position) || left.id.localeCompare(right.id))
        .map((row) => row.payload as T)
    const flowRunRequests = artifacts<FlowRunRequestResponse>('flow_run_requests')
    const flowLaunches = artifacts<FlowLaunchResponse>('flow_launches')
    const proposedPlans = artifacts<ProposedPlanArtifactResponse>('proposed_plans')

    // Turns before the oldest loaded page stay hidden until that page loads; pinned ones show.
    const pinned = new Set(state.pinnedTurnIds)
    const isVisible = (row: RowOf<'turn'>) => state.olderCursor === null
        || state.windowStart === null
        || pinned.has(row.id)
        || (row.position !== undefined && compareRevisions(row.position, state.windowStart) >= 0)
    const turnsById: Record<string, ConversationTurnResponse> = {}
    const orderedTurnIds = turnRows
        .filter(isVisible)
        .sort((left, right) => compareOptionalDecimals(left.position, right.position) || left.id.localeCompare(right.id))
        .map((row) => {
            turnsById[row.id] = state.streamTurnsById[row.id] ?? row.payload!
            return row.id
        })
    Object.values(state.streamTurnsById).forEach((turn) => {
        if (!state.rows[turnKey(conversationId, turn.id)]) {
            turnsById[turn.id] = turn
            orderedTurnIds.push(turn.id)
        }
    })

    const planIdBySegmentId = new Map(proposedPlans.flatMap((plan) => plan.source_segment_id ? [[plan.source_segment_id, plan.id]] : []))
    const ordered: Array<{ segment: ConversationSegmentResponse; order: string }> = segmentRows
        .map((row) => ({ segment: state.streamSegmentsById[row.id] ?? row.payload!, order: row.order ?? '0' }))
    Object.values(state.streamSegmentsById).forEach((segment) => {
        if (!state.rows[segmentKey(conversationId, segment.id)]) {
            ordered.push({ segment, order: Number.isSafeInteger(segment.order) && segment.order >= 0 ? String(segment.order) : '0' })
        }
    })
    ordered.sort((left, right) => compareRevisions(left.order, right.order)
        || left.segment.timestamp.localeCompare(right.segment.timestamp)
        || left.segment.id.localeCompare(right.segment.id))
    const segmentsById: Record<string, ConversationSegmentResponse> = {}
    const orderedSegmentIdsByTurnId: Record<string, string[]> = {}
    const segmentsByTurnId: Record<string, ConversationSegmentResponse[]> = {}
    ordered.forEach(({ segment: committed }) => {
        if (!turnsById[committed.turn_id]) return
        const linkedPlanId = committed.artifact_id ? undefined : planIdBySegmentId.get(committed.id)
        const segment = linkedPlanId ? { ...committed, artifact_id: linkedPlanId } : committed
        segmentsById[segment.id] = segment
        ;(orderedSegmentIdsByTurnId[segment.turn_id] ??= []).push(segment.id)
        ;(segmentsByTurnId[segment.turn_id] ??= []).push(segment)
    })

    const timelineEntryIds: string[] = []
    const timelineEntriesById: Record<string, ConversationTimelineEntry> = {}
    const timelineEntryIdsByTurnId: Record<string, string[]> = {}
    orderedTurnIds.forEach((turnId) => {
        const turn = turnsById[turnId]
        const segments = segmentsByTurnId[turnId] ?? []
        const previousIds = previous?.timelineEntryIdsByTurnId[turnId]
        const entries = previous && previousIds && previous.turnsById[turnId] === turn && sameTurnSegments(previous, turnId, segments)
            ? previousIds.map((id) => previous.timelineEntriesById[id])
            : buildConversationTimelineEntriesForTurn(turn, segments)
        timelineEntryIdsByTurnId[turnId] = entries.map((entry) => {
            timelineEntriesById[entry.id] = entry
            timelineEntryIds.push(entry.id)
            return entry.id
        })
    })

    const payload = conversation?.payload ?? null
    return {
        ...state,
        revision: conversation?.revision ?? '0',
        settingsRevision: payload?.settings_revision ?? '0',
        latestRevision,
        deleted: conversation?.deleted === true,
        model_settings_view: payload?.model_settings_view,
        conversation_id: conversationId,
        conversation_handle: payload?.handle ?? null,
        project_path: state.projectPath ?? payload?.project_path ?? previous?.project_path ?? '',
        chat_mode: payload?.chat_mode ?? 'chat',
        provider: payload?.provider ?? null,
        model: payload?.model ?? null,
        reasoning_effort: payload?.reasoning_effort ?? null,
        title: payload?.title ?? 'New thread',
        created_at: payload?.created_at ?? '',
        updated_at: payload?.updated_at ?? '',
        orderedTurnIds,
        turnsById,
        orderedSegmentIdsByTurnId,
        segmentsById,
        flowRunRequestIds: flowRunRequests.map((request) => request.id),
        flowRunRequestsById: Object.fromEntries(flowRunRequests.map((request) => [request.id, request])),
        flowLaunchIds: flowLaunches.map((launch) => launch.id),
        flowLaunchesById: Object.fromEntries(flowLaunches.map((launch) => [launch.id, launch])),
        proposedPlanIds: proposedPlans.map((plan) => plan.id),
        proposedPlansById: Object.fromEntries(proposedPlans.map((plan) => [plan.id, plan])),
        timelineEntryIds,
        timelineEntriesById,
        timelineEntryIdsByTurnId,
    }
}

function pageStart(page: ConversationPage): string | null {
    const pageTurnIds = new Set(page.turn_ids)
    return page.rows.reduce<string | null>((start, row) => (
        row.kind === 'turn' && row.position !== undefined && pageTurnIds.has(row.id)
            && (start === null || compareRevisions(row.position, start) < 0)
            ? row.position
            : start
    ), null)
}

/**
 * A `window` page replaces the loaded window: history and cursors go, but rows a live batch
 * applied after the page's read survive it. An `older` page only adds rows, by revision, and
 * never moves the live cursor.
 */
export function applyConversationPage(
    record: NormalizedConversationRecord | undefined,
    page: ConversationPage,
    mode: 'window' | 'older',
    projectPath?: string,
): NormalizedConversationRecord | undefined {
    const sameGeneration = record?.generation === page.generation
    if (mode === 'older') {
        if (!record || !sameGeneration || record.deleted) {
            return record
        }
        const start = pageStart(page)
        return deriveConversationRecord(page.conversation_id, {
            ...windowOf(record),
            rows: mergeConversationRows(record.rows, page.rows),
            olderCursor: page.older_cursor,
            windowStart: record.windowStart === null || (start !== null && compareRevisions(start, record.windowStart) < 0)
                ? start
                : record.windowStart,
        }, record)
    }
    const newer = record && sameGeneration
        ? Object.values(record.rows).filter((row) => compareRevisions(row.revision, page.watermark) > 0)
        : []
    const rows = mergeConversationRows(mergeConversationRows({}, page.rows), newer)
    const keepsWatermark = record && sameGeneration && !record.resetting && record.watermark !== null
        && compareRevisions(record.watermark, page.watermark) > 0
    return deriveConversationRecord(page.conversation_id, dropChildrenOfDeletedConversation(page.conversation_id, {
        generation: page.generation,
        projectPath: projectPath ?? record?.projectPath ?? null,
        watermark: keepsWatermark ? record.watermark : page.watermark,
        resetting: false,
        olderCursor: page.older_cursor,
        windowStart: pageStart(page),
        pinnedTurnIds: page.pinned_turn_ids,
        rows,
        ...(sameGeneration ? settleStreamOverlay(record!, rows, page.rows) : { streamTurnsById: {}, streamSegmentsById: {} }),
    }), sameGeneration ? record : undefined)
}

/**
 * The stream overlay once committed rows land: a committed segment replaces its streamed copy,
 * a finished turn drops everything it streamed, and a running turn's commit without its text
 * keeps the text it showed.
 */
function settleStreamOverlay(
    record: NormalizedConversationRecord,
    rows: ConversationRows,
    incoming: ConversationRow[],
): Pick<ConversationWindowState, 'streamTurnsById' | 'streamSegmentsById'> {
    let streamTurnsById = record.streamTurnsById
    let streamSegmentsById = record.streamSegmentsById
    incoming.forEach((row) => {
        if (rows[conversationRowKey(row)] !== row) return
        if (row.kind === 'segment' && streamSegmentsById[row.id]) {
            streamSegmentsById = { ...streamSegmentsById }
            delete streamSegmentsById[row.id]
        }
        if (row.kind === 'turn') {
            // A finished turn's committed segments replace everything it streamed.
            if (row.deleted || (row.payload?.status !== 'pending' && row.payload?.status !== 'streaming')) {
                const kept = Object.values(streamSegmentsById).filter((segment) => segment.turn_id !== row.id)
                if (kept.length !== Object.keys(streamSegmentsById).length) {
                    streamSegmentsById = Object.fromEntries(kept.map((segment) => [segment.id, segment]))
                }
            }
            // A running turn's commit without its streamed text keeps the text it showed.
            const shown = streamTurnsById[row.id] ?? record.turnsById[row.id]
            const kept = row.payload && shown ? sanitizeStreamingTurnUpsert(shown, row.payload) : row.payload
            if (kept !== row.payload || streamTurnsById[row.id]) {
                streamTurnsById = { ...streamTurnsById }
                if (kept && kept !== row.payload) streamTurnsById[row.id] = kept
                else delete streamTurnsById[row.id]
            }
        }
    })
    return { streamTurnsById, streamSegmentsById }
}

/**
 * Applies a whole batch by row revision. Only a `live` batch advances the catch-up cursor; a
 * `mutation` result may omit concurrent changes. A mutation can start a conversation the client
 * has not loaded, such as a new thread.
 */
export function applyConversationBatch(
    record: NormalizedConversationRecord | undefined,
    conversationId: string,
    batch: ConversationBatch,
    kind: 'live' | 'mutation',
    projectPath?: string,
): NormalizedConversationRecord | undefined {
    if (!record) {
        return kind === 'mutation'
            ? deriveConversationRecord(conversationId, dropChildrenOfDeletedConversation(conversationId, {
                ...emptyWindow(batch.generation, projectPath ?? null),
                rows: mergeConversationRows({}, batch.rows),
            }))
            : undefined
    }
    if (record.generation !== batch.generation || (kind === 'live' && record.resetting)) {
        return record
    }
    const rows = mergeConversationRows(record.rows, batch.rows)
    const watermark = kind === 'live' && (record.watermark === null || compareRevisions(batch.watermark, record.watermark) > 0)
        ? batch.watermark
        : record.watermark
    if (rows === record.rows && watermark === record.watermark) {
        return record
    }
    const { streamTurnsById, streamSegmentsById } = settleStreamOverlay(record, rows, batch.rows)
    return deriveConversationRecord(conversationId, dropChildrenOfDeletedConversation(conversationId, {
        ...windowOf(record),
        rows,
        watermark,
        streamTurnsById,
        streamSegmentsById,
    }), record)
}

/**
 * The thread-list summaries a batch of conversation rows changes, by revision; tombstones remove.
 * A summary is filed under `scopePath` when the rows answer a request for that project, else
 * where it is already listed, and only otherwise under the row's own project path.
 */
function mergeSummaryRows(current: ProjectConversationCacheState, rows: ConversationRow[], scopePath?: string): ProjectConversationCacheState {
    let lists = current.summariesByProjectPath
    let tombstones = current.summaryTombstones ?? {}
    const pathById = new Map<string, string>()
    Object.entries(lists).forEach(([path, items]) => items.forEach((item) => pathById.set(item.conversation_id, path)))
    rows.forEach((row) => {
        if (row.kind !== 'conversation') return
        const tombstone = tombstones[row.id]
        const path = pathById.get(row.id)
        const existing = path === undefined ? undefined : lists[path].find((item) => item.conversation_id === row.id)
        if (
            (tombstone !== undefined && compareRevisions(row.revision, tombstone) <= 0)
            || (existing && compareRevisions(row.revision, existing.revision) <= 0)
        ) {
            return
        }
        if (lists === current.summariesByProjectPath) lists = { ...lists }
        if (path !== undefined) {
            lists[path] = lists[path].filter((item) => item.conversation_id !== row.id)
            pathById.delete(row.id)
        }
        if (row.deleted || !row.payload) {
            tombstones = { ...tombstones, [row.id]: row.revision }
            return
        }
        const filedPath = scopePath ?? path ?? row.payload.project_path
        let summary = { ...conversationSummaryFromRow({ ...row, payload: row.payload }), project_path: filedPath }
        // A conversation-scope row doesn't carry the standing role the summary feed listed.
        const role = row.payload.standing_role === undefined ? existing?.standing_role : null
        if (role) summary = { ...summary, title: role.title, standing_role: role }
        lists[summary.project_path] = upsertConversationSummary(lists[summary.project_path] ?? [], summary)
        pathById.set(row.id, summary.project_path)
    })
    return lists === current.summariesByProjectPath && tombstones === (current.summaryTombstones ?? {})
        ? current
        : { ...current, summariesByProjectPath: lists, summaryTombstones: tombstones }
}

function withConversationRecord(
    current: ProjectConversationCacheState,
    record: NormalizedConversationRecord | undefined,
    rows: ConversationRow[],
): ProjectConversationCacheState {
    if (!record || current.conversationsById[record.conversation_id] === record) {
        return current
    }
    // An empty window doesn't show that a conversation gone after a reset exists again.
    if (current.unavailableConversationIds?.[record.conversation_id] && !current.conversationsById[record.conversation_id]
        && !rows.some((row) => row.kind === 'conversation' && !row.deleted)) {
        return current
    }
    const unavailable = { ...current.unavailableConversationIds }
    if (record.deleted) unavailable[record.conversation_id] = true
    else delete unavailable[record.conversation_id]
    return mergeSummaryRows({
        ...current,
        conversationsById: { ...current.conversationsById, [record.conversation_id]: record },
        unavailableConversationIds: unavailable,
    }, rows, record.projectPath ?? undefined)
}

export function applyConversationPageToCache(
    current: ProjectConversationCacheState,
    page: ConversationPage,
    mode: 'window' | 'older',
    projectPath?: string,
): ProjectConversationCacheState {
    return withConversationRecord(
        current,
        applyConversationPage(current.conversationsById[page.conversation_id], page, mode, projectPath),
        page.rows,
    )
}

export function applyConversationBatchToCache(
    current: ProjectConversationCacheState,
    conversationId: string,
    batch: ConversationBatch,
    kind: 'live' | 'mutation',
    projectPath?: string,
): ProjectConversationCacheState {
    return withConversationRecord(
        current,
        applyConversationBatch(current.conversationsById[conversationId], conversationId, batch, kind, projectPath),
        batch.rows,
    )
}

/**
 * After a live reset: the loaded history, stream overlay and both cursors are invalid, so only the
 * conversation row stays, and live batches and deltas wait for the next window. Pending sends
 * live in the composer's session state, so their identities survive.
 */
export function resetConversationInCache(current: ProjectConversationCacheState, conversationId: string): ProjectConversationCacheState {
    const record = current.conversationsById[conversationId]
    if (!record) {
        return current
    }
    const key = conversationRowKey({ kind: 'conversation', conversation_id: conversationId, id: conversationId })
    const conversation = record.rows[key]
    const cleared = deriveConversationRecord(conversationId, {
        ...emptyWindow(record.generation, record.projectPath),
        resetting: true,
        rows: conversation ? { [key]: conversation } : {},
    }, record)
    return { ...current, conversationsById: { ...current.conversationsById, [conversationId]: cleared } }
}

/** A conversation gone after a reset: its cache is discarded and it shows as unavailable. */
export function markConversationUnavailableInCache(current: ProjectConversationCacheState, conversationId: string): ProjectConversationCacheState {
    return {
        ...removeConversationFromCache(current, conversationId),
        unavailableConversationIds: { ...current.unavailableConversationIds, [conversationId]: true },
    }
}

/**
 * Replaces the summaries in the snapshot's scope (one project, or every project) atomically.
 * A summary missing from it is dropped unless a live row newer than its read put it there, and
 * other scopes and open transcripts are untouched. An HTTP snapshot only lowers the live cursor,
 * so the feed still catches up from it. `projectPath` is the project the client asked for, which
 * scopes and files the list whatever spelling of it the server returns.
 */
export function applySummarySnapshotToCache(
    current: ProjectConversationCacheState,
    snapshot: ConversationSummarySnapshot,
    source: 'live' | 'http',
    projectPath: string | null = snapshot.project_path,
): ProjectConversationCacheState {
    const authority = current.summaryAuthority
    if (source === 'http' && authority && (authority.generation !== snapshot.generation
        || compareRevisions(snapshot.watermark, authority.watermark) < 0)) {
        return current
    }
    const inScope = (path: string) => projectPath === null || path === projectPath
    const generation = current.summaryGeneration ?? current.summaryCursor?.generation ?? null
    // Revisions from another store generation say nothing about this one: every summary row,
    // tombstone and the cursor of the old generation go; open transcripts stay.
    const changedGeneration = generation !== null && generation !== snapshot.generation
    const cursor = changedGeneration ? null : current.summaryCursor
    const base = changedGeneration
        ? { ...current, summaryTombstones: {}, summariesByProjectPath: {}, summaryCursor: null }
        : current
    const merged = mergeSummaryRows(base, snapshot.rows, projectPath ?? undefined)
    const listed = new Set(snapshot.rows.map((row) => row.id))
    const lists = { ...merged.summariesByProjectPath }
    Object.keys(lists).forEach((path) => {
        if (inScope(path)) {
            lists[path] = lists[path].filter((item) => (
                listed.has(item.conversation_id) || compareRevisions(item.revision, snapshot.watermark) > 0
            ))
        }
    })
    if (projectPath !== null) {
        lists[projectPath] ??= []
    }
    // The cursor follows the all-projects feed: a project's snapshot covers no other project's
    // rows, so it never starts the cursor, and an HTTP snapshot only lowers it.
    const snapshotCursor = { generation: snapshot.generation, watermark: snapshot.watermark }
    const lowered = cursor && compareRevisions(cursor.watermark, snapshot.watermark) <= 0 ? cursor : snapshotCursor
    const summaryCursor = source === 'live' && projectPath === null
        ? snapshotCursor
        : cursor ? lowered : projectPath === null ? snapshotCursor : null
    return {
        ...merged,
        summariesByProjectPath: lists,
        summaryGeneration: snapshot.generation,
        summaryCursor,
        summaryAuthority: source === 'live' && projectPath === null
            ? { generation: snapshot.generation, watermark: snapshot.watermark }
            : changedGeneration ? null : authority ?? null,
    }
}

export function applySummaryBatchToCache(current: ProjectConversationCacheState, batch: ConversationBatch): ProjectConversationCacheState {
    const cursor = current.summaryCursor
    const advances = !cursor || cursor.generation !== batch.generation || compareRevisions(batch.watermark, cursor.watermark) > 0
    return {
        ...mergeSummaryRows(current, batch.rows),
        summaryGeneration: batch.generation,
        summaryCursor: advances ? { generation: batch.generation, watermark: batch.watermark } : cursor,
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

export type ApplyTransientConversationEventResult =
    | {
        status: 'applied'
        record: NormalizedConversationRecord
        cache: ProjectConversationCacheState
    }
    | {
        status: 'dropped'
        cache: ProjectConversationCacheState
    }

/**
 * Apply a transient stream delta over the committed rows. Transients update the visible
 * turn/segment state but never change a row or a revision, never touch summaries or artifacts,
 * and are simply dropped when stale or when no committed record exists yet; the turn's
 * committed rows restore durable state.
 */
export function applyTransientConversationEventToCache(
    current: ProjectConversationCacheState,
    event: ConversationStreamDeltaEventResponse,
): ApplyTransientConversationEventResult {
    // Staleness is per turn: other commits during the turn raise revisions
    // but don't make its live updates stale.
    const record = current.conversationsById[event.conversation_id]
    const turnStatus = record?.turnsById[event.turn_id]?.status
    const lastSequence = current.streamSequenceByTurnId?.[event.turn_id] ?? 0
    if (
        !record
        || record.deleted
        || record.resetting
        || turnStatus === 'complete'
        || turnStatus === 'failed'
        || event.stream_sequence <= lastSequence
    ) {
        return { status: 'dropped', cache: current }
    }
    let state = windowOf(record)
    if (event.delta_kind === 'turn_delta' && event.turn) {
        const turn = sanitizeStreamingTurnUpsert(record.turnsById[event.turn.id] || null, event.turn)
        state = { ...state, streamTurnsById: { ...state.streamTurnsById, [turn.id]: turn } }
    } else if (event.delta_kind === 'segment_delta' && event.segment && !record.rows[segmentKey(record.conversation_id, event.segment.id)]?.deleted) {
        state = { ...state, streamSegmentsById: { ...state.streamSegmentsById, [event.segment.id]: event.segment } }
    } else if (event.delta_kind === 'token_usage' && event.token_usage && record.turnsById[event.turn_id]) {
        const turn = { ...record.turnsById[event.turn_id], token_usage: event.token_usage }
        state = { ...state, streamTurnsById: { ...state.streamTurnsById, [turn.id]: turn } }
    } else {
        return { status: 'dropped', cache: current }
    }
    const mergedRecord = deriveConversationRecord(record.conversation_id, state, record)
    return {
        status: 'applied',
        record: mergedRecord,
        cache: {
            ...current,
            conversationsById: {
                ...current.conversationsById,
                [event.conversation_id]: mergedRecord,
            },
            streamSequenceByTurnId: {
                ...current.streamSequenceByTurnId,
                [event.turn_id]: event.stream_sequence,
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
        ...current,
        conversationsById: nextConversationsById,
        summariesByProjectPath: nextSummariesByProjectPath,
    }
}
