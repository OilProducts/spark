import { create } from 'zustand'

import type { PendingInterviewGate, TimelineEventEntry } from '../model/shared'
import {
    buildJournalPendingInterviewGates,
    buildRetryCorrelationEntityKeys,
    buildTimelineTypeOptions,
    pendingInterviewGateDedupeKey,
    timelineEntityKey,
    toPendingInterviewGateFromEvent,
} from '../model/timelineModel'

type RunJournalResourceStatus = 'idle' | 'loading' | 'ready' | 'error'
type RunJournalLiveStatus = 'idle' | 'connecting' | 'live' | 'degraded'

const RUN_JOURNAL_SEGMENT_CAPACITY = 100

export interface RunJournalSegment {
    id: string
    role: 'latest' | 'older' | 'live'
    cursor: number | null
    hasOlder: boolean
    newestSequence: number | null
    oldestSequence: number | null
    entries: TimelineEventEntry[]
}

export type RunJournalMutation =
    | {
        kind: 'merge_latest'
        revision: number
    }
    | {
        kind: 'append_older'
        revision: number
    }
    | {
        kind: 'append_live'
        revision: number
        entry: TimelineEventEntry
        appendedAsNewest: boolean
        introducedRetryCorrelation: boolean
    }
    | null

type RunJournalRebuildMutationKind = Extract<
    NonNullable<RunJournalMutation>,
    { kind: 'merge_latest' | 'append_older' }
>['kind']

export interface RunJournalStateEntry {
    segments: RunJournalSegment[]
    oldestSequence: number | null
    newestSequence: number | null
    loadedEntryCount: number
    latestEntry: TimelineEventEntry | null
    latestRetryEntry: TimelineEventEntry | null
    timelineTypeOptions: string[]
    pendingInterviewGates: PendingInterviewGate[]
    hasOlder: boolean
    status: RunJournalResourceStatus
    error: string | null
    isLoadingOlder: boolean
    liveStatus: RunJournalLiveStatus
    liveError: string | null
    revision: number
    lastMutation: RunJournalMutation
    _knownSequences: Set<number>
    _retryCorrelationEntityKeys: Set<string>
    _closedInterviewEntityKeys: Set<string>
    _pendingInterviewGateKeys: Set<string>
}

interface RunJournalStoreState {
    byRunId: Record<string, RunJournalStateEntry>
    patchRun: (runId: string, patch: Partial<RunJournalStateEntry>) => void
    mergeLatestPage: (runId: string, page: {
        entries: TimelineEventEntry[]
        oldestSequence: number | null
        newestSequence: number | null
        hasOlder: boolean
    }) => void
    appendOlderPage: (runId: string, page: {
        entries: TimelineEventEntry[]
        oldestSequence: number | null
        newestSequence: number | null
        hasOlder: boolean
    }) => void
    appendLiveEntry: (runId: string, entry: TimelineEventEntry) => void
    clearRun: (runId: string) => void
}

function createDefaultRunJournalState(): RunJournalStateEntry {
    return {
        segments: [],
        oldestSequence: null,
        newestSequence: null,
        loadedEntryCount: 0,
        latestEntry: null,
        latestRetryEntry: null,
        timelineTypeOptions: [],
        pendingInterviewGates: [],
        hasOlder: false,
        status: 'idle',
        error: null,
        isLoadingOlder: false,
        liveStatus: 'idle',
        liveError: null,
        revision: 0,
        lastMutation: null,
        _knownSequences: new Set<number>(),
        _retryCorrelationEntityKeys: new Set<string>(),
        _closedInterviewEntityKeys: new Set<string>(),
        _pendingInterviewGateKeys: new Set<string>(),
    }
}

const resolveRunJournalState = (
    byRunId: Record<string, RunJournalStateEntry>,
    runId: string,
): RunJournalStateEntry => byRunId[runId] ?? createDefaultRunJournalState()

const dedupeAndSortEntries = (entries: TimelineEventEntry[]): TimelineEventEntry[] => {
    const bySequence = new Map<number, TimelineEventEntry>()
    for (const entry of entries) {
        bySequence.set(entry.sequence, entry)
    }
    return Array.from(bySequence.values()).sort((left, right) => right.sequence - left.sequence)
}

export const flattenRunJournalSegments = (segments: RunJournalSegment[]): TimelineEventEntry[] => (
    dedupeAndSortEntries(segments.flatMap((segment) => segment.entries))
)

export function* iterateRunJournalEntries(
    segments: RunJournalSegment[],
): IterableIterator<TimelineEventEntry> {
    for (const segment of segments) {
        for (const entry of segment.entries) {
            yield entry
        }
    }
}

const buildSegmentId = (
    role: RunJournalSegment['role'],
    entries: TimelineEventEntry[],
    cursor: number | null,
): string => {
    const newestSequence = entries[0]?.sequence ?? 'none'
    const oldestSequence = entries.at(-1)?.sequence ?? 'none'
    return `${role}-${cursor ?? 'none'}-${newestSequence}-${oldestSequence}`
}

const createSegment = (
    role: RunJournalSegment['role'],
    entries: TimelineEventEntry[],
    cursor: number | null,
    hasOlder = false,
): RunJournalSegment => {
    const normalizedEntries = dedupeAndSortEntries(entries)
    return {
        id: buildSegmentId(role, normalizedEntries, cursor),
        role,
        cursor,
        hasOlder,
        newestSequence: normalizedEntries[0]?.sequence ?? null,
        oldestSequence: normalizedEntries.at(-1)?.sequence ?? null,
        entries: normalizedEntries,
    }
}

const createSegments = (
    role: RunJournalSegment['role'],
    entries: TimelineEventEntry[],
    cursor: number | null,
    hasOlder = false,
): RunJournalSegment[] => {
    const normalizedEntries = dedupeAndSortEntries(entries)
    if (normalizedEntries.length === 0) {
        return []
    }

    const segments: RunJournalSegment[] = []
    for (let startIndex = 0; startIndex < normalizedEntries.length; startIndex += RUN_JOURNAL_SEGMENT_CAPACITY) {
        const chunk = normalizedEntries.slice(startIndex, startIndex + RUN_JOURNAL_SEGMENT_CAPACITY)
        const isLastChunk = startIndex + RUN_JOURNAL_SEGMENT_CAPACITY >= normalizedEntries.length
        segments.push(createSegment(role, chunk, cursor, isLastChunk ? hasOlder : false))
    }
    return segments
}

const summarizeSegments = (segments: RunJournalSegment[]) => {
    let loadedEntryCount = 0
    let latestEntry: TimelineEventEntry | null = null
    let latestRetryEntry: TimelineEventEntry | null = null
    let newestSequence: number | null = null
    let oldestSequence: number | null = null

    for (const segment of segments) {
        if (segment.entries.length === 0) {
            continue
        }
        if (!latestEntry) {
            latestEntry = segment.entries[0] ?? null
            newestSequence = latestEntry?.sequence ?? null
        }
        loadedEntryCount += segment.entries.length
        oldestSequence = segment.entries.at(-1)?.sequence ?? oldestSequence
        if (!latestRetryEntry) {
            latestRetryEntry = segment.entries.find((entry) => entry.type === 'StageRetrying') ?? null
        }
    }

    return {
        loadedEntryCount,
        latestEntry,
        latestRetryEntry,
        newestSequence,
        oldestSequence,
    }
}

const buildKnownSequenceSet = (segments: RunJournalSegment[]) => {
    const knownSequences = new Set<number>()
    for (const entry of iterateRunJournalEntries(segments)) {
        knownSequences.add(entry.sequence)
    }
    return knownSequences
}

const buildPendingInterviewMetadata = (segments: RunJournalSegment[]) => {
    const pendingMetadata = buildJournalPendingInterviewGates(iterateRunJournalEntries(segments))
    return {
        pendingInterviewGates: pendingMetadata.pendingGates,
        closedInterviewEntityKeys: pendingMetadata.closedEntityKeys,
        pendingInterviewGateKeys: pendingMetadata.pendingGateKeys,
    }
}

const resolveHasOlderFromLoadedRange = (
    segments: RunJournalSegment[],
    fallbackHasOlder: boolean,
): boolean => {
    const oldestPersistedSegment = segments
        .filter((segment) => segment.role !== 'live' && segment.entries.length > 0)
        .reduce<RunJournalSegment | null>((oldest, segment) => {
            const segmentOldestSequence = segment.oldestSequence ?? Number.POSITIVE_INFINITY
            const oldestSequence = oldest?.oldestSequence ?? Number.POSITIVE_INFINITY
            return segmentOldestSequence < oldestSequence ? segment : oldest
        }, null)
    if (!oldestPersistedSegment) {
        return fallbackHasOlder
    }
    return oldestPersistedSegment.hasOlder
}

const rebuildRunJournalState = (
    state: RunJournalStateEntry,
    segments: RunJournalSegment[],
    patch: Partial<RunJournalStateEntry>,
    mutationKind: RunJournalRebuildMutationKind,
): RunJournalStateEntry => {
    const summary = summarizeSegments(segments)
    const pendingInterviewMetadata = buildPendingInterviewMetadata(segments)
    const revision = state.revision + 1
    const lastMutation: RunJournalMutation = mutationKind === 'merge_latest'
        ? {
            kind: 'merge_latest',
            revision,
        }
        : {
            kind: 'append_older',
            revision,
        }
    return {
        ...state,
        ...patch,
        segments,
        newestSequence: summary.newestSequence,
        oldestSequence: summary.oldestSequence,
        loadedEntryCount: summary.loadedEntryCount,
        latestEntry: summary.latestEntry,
        latestRetryEntry: summary.latestRetryEntry,
        timelineTypeOptions: buildTimelineTypeOptions(iterateRunJournalEntries(segments)),
        pendingInterviewGates: pendingInterviewMetadata.pendingInterviewGates,
        revision,
        lastMutation,
        _knownSequences: buildKnownSequenceSet(segments),
        _retryCorrelationEntityKeys: buildRetryCorrelationEntityKeys(iterateRunJournalEntries(segments)),
        _closedInterviewEntityKeys: pendingInterviewMetadata.closedInterviewEntityKeys,
        _pendingInterviewGateKeys: pendingInterviewMetadata.pendingInterviewGateKeys,
    }
}

const appendPendingInterviewGate = (
    state: RunJournalStateEntry,
    entry: TimelineEventEntry,
) => {
    const entityKey = timelineEntityKey(entry) || `event:${entry.id}`
    const nextClosedInterviewEntityKeys = new Set(state._closedInterviewEntityKeys)
    let nextPendingInterviewGateKeys = new Set(state._pendingInterviewGateKeys)
    let nextPendingInterviewGates = state.pendingInterviewGates

    if (entry.category !== 'interview') {
        return {
            closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
            pendingInterviewGateKeys: nextPendingInterviewGateKeys,
            pendingInterviewGates: nextPendingInterviewGates,
        }
    }

    if (nextClosedInterviewEntityKeys.has(entityKey)) {
        return {
            closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
            pendingInterviewGateKeys: nextPendingInterviewGateKeys,
            pendingInterviewGates: nextPendingInterviewGates,
        }
    }

    if (entry.type === 'InterviewCompleted' || entry.type === 'InterviewTimeout') {
        nextClosedInterviewEntityKeys.add(entityKey)
        nextPendingInterviewGates = nextPendingInterviewGates.filter((gate) => (
            (gate.nodeId ?? '') !== (entry.nodeId ?? '')
            || gate.stageIndex !== entry.stageIndex
            || gate.sourceScope !== entry.sourceScope
            || (gate.sourceParentNodeId ?? null) !== (entry.sourceParentNodeId ?? null)
            || (gate.sourceFlowName ?? null) !== (entry.sourceFlowName ?? null)
        ))
        nextPendingInterviewGateKeys = new Set(
            nextPendingInterviewGates.map((gate) => pendingInterviewGateDedupeKey(
                gate.sourceScope,
                gate.sourceParentNodeId,
                gate.sourceFlowName,
                gate.nodeId,
                gate.prompt,
            )),
        )
        return {
            closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
            pendingInterviewGateKeys: nextPendingInterviewGateKeys,
            pendingInterviewGates: nextPendingInterviewGates,
        }
    }

    const nextGate = toPendingInterviewGateFromEvent(entry)
    if (!nextGate) {
        return {
            closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
            pendingInterviewGateKeys: nextPendingInterviewGateKeys,
            pendingInterviewGates: nextPendingInterviewGates,
        }
    }

    const nextGateKey = pendingInterviewGateDedupeKey(
        nextGate.sourceScope,
        nextGate.sourceParentNodeId,
        nextGate.sourceFlowName,
        nextGate.nodeId,
        nextGate.prompt,
    )
    if (nextPendingInterviewGateKeys.has(nextGateKey)) {
        return {
            closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
            pendingInterviewGateKeys: nextPendingInterviewGateKeys,
            pendingInterviewGates: nextPendingInterviewGates,
        }
    }

    nextPendingInterviewGateKeys.add(nextGateKey)
    nextPendingInterviewGates = [nextGate, ...nextPendingInterviewGates]
    return {
        closedInterviewEntityKeys: nextClosedInterviewEntityKeys,
        pendingInterviewGateKeys: nextPendingInterviewGateKeys,
        pendingInterviewGates: nextPendingInterviewGates,
    }
}

export const useRunJournalStore = create<RunJournalStoreState>()((set) => ({
    byRunId: {},
    patchRun: (runId, patch) =>
        set((state) => ({
            byRunId: {
                ...state.byRunId,
                [runId]: {
                    ...resolveRunJournalState(state.byRunId, runId),
                    ...patch,
                },
            },
        })),
    mergeLatestPage: (runId, page) =>
        set((state) => {
            const current = resolveRunJournalState(state.byRunId, runId)
            const latestSegments = createSegments('latest', page.entries, page.oldestSequence, page.hasOlder)
            const latestNewestSequence = latestSegments[0]?.newestSequence ?? null
            const latestOldestSequence = latestSegments.at(-1)?.oldestSequence ?? null
            const preservedLiveSegments = current.segments
                .filter((segment) => segment.role === 'live')
                .map((segment) => createSegment(
                    'live',
                    latestNewestSequence === null
                        ? segment.entries
                        : segment.entries.filter((entry) => entry.sequence > latestNewestSequence),
                    segment.cursor,
                    false,
                ))
                .filter((segment) => segment.entries.length > 0)
            const preservedOlderSegments = current.segments
                .filter((segment) => segment.role === 'older')
                .map((segment) => createSegment(
                    'older',
                    latestOldestSequence === null
                        ? segment.entries
                        : segment.entries.filter((entry) => entry.sequence < latestOldestSequence),
                    segment.cursor,
                    segment.hasOlder,
                ))
                .filter((segment) => segment.entries.length > 0)
            const nextSegments = [
                ...preservedLiveSegments,
                ...latestSegments,
                ...preservedOlderSegments,
            ]
            return {
                byRunId: {
                    ...state.byRunId,
                    [runId]: rebuildRunJournalState(current, nextSegments, {
                        hasOlder: resolveHasOlderFromLoadedRange(nextSegments, page.hasOlder),
                        status: 'ready',
                        error: null,
                        isLoadingOlder: false,
                    }, 'merge_latest'),
                },
            }
        }),
    appendOlderPage: (runId, page) =>
        set((state) => {
            const current = resolveRunJournalState(state.byRunId, runId)
            const olderEntries = page.entries.filter((entry) => !current._knownSequences.has(entry.sequence))
            const olderSegments = createSegments('older', olderEntries, page.oldestSequence, page.hasOlder)
            const retainedSegments = current.segments.filter((segment) => (
                !(segment.role === 'older' && segment.cursor === page.oldestSequence)
            ))
            const nextSegments = olderSegments.length > 0
                ? [...retainedSegments, ...olderSegments]
                : retainedSegments
            return {
                byRunId: {
                    ...state.byRunId,
                    [runId]: rebuildRunJournalState(current, nextSegments, {
                        hasOlder: resolveHasOlderFromLoadedRange(nextSegments, page.hasOlder),
                        status: 'ready',
                        error: null,
                        isLoadingOlder: false,
                    }, 'append_older'),
                },
            }
        }),
    appendLiveEntry: (runId, entry) =>
        set((state) => {
            const current = resolveRunJournalState(state.byRunId, runId)
            if (current._knownSequences.has(entry.sequence)) {
                return state
            }

            const currentLiveSegment = current.segments[0]?.role === 'live' ? current.segments[0] : null
            const hasLiveCapacity = Boolean(currentLiveSegment && currentLiveSegment.entries.length < RUN_JOURNAL_SEGMENT_CAPACITY)
            const nextLiveSegment = createSegment(
                'live',
                hasLiveCapacity && currentLiveSegment ? [entry, ...currentLiveSegment.entries] : [entry],
                entry.sequence,
                false,
            )
            const retainedSegments = hasLiveCapacity && currentLiveSegment
                ? current.segments.slice(1)
                : current.segments
            const nextSegments = [nextLiveSegment, ...retainedSegments]
            const nextKnownSequences = new Set(current._knownSequences)
            nextKnownSequences.add(entry.sequence)

            const entityKey = timelineEntityKey(entry)
            const introducesRetryCorrelation = Boolean(
                entityKey
                && (entry.type === 'StageRetrying' || typeof entry.payload.attempt === 'number')
                && !current._retryCorrelationEntityKeys.has(entityKey),
            )
            const nextRetryCorrelationEntityKeys = new Set(current._retryCorrelationEntityKeys)
            if (introducesRetryCorrelation && entityKey) {
                nextRetryCorrelationEntityKeys.add(entityKey)
            }

            const pendingInterviewMetadata = appendPendingInterviewGate(current, entry)
            const nextTimelineTypeOptions = current.timelineTypeOptions.includes(entry.type)
                ? current.timelineTypeOptions
                : [...current.timelineTypeOptions, entry.type].sort((left, right) => left.localeCompare(right))
            const appendedAsNewest = current.newestSequence === null || entry.sequence > current.newestSequence
            const revision = current.revision + 1

            return {
                byRunId: {
                    ...state.byRunId,
                    [runId]: {
                        ...current,
                        segments: nextSegments,
                        newestSequence: appendedAsNewest ? entry.sequence : current.newestSequence,
                        oldestSequence: current.oldestSequence ?? entry.sequence,
                        loadedEntryCount: current.loadedEntryCount + 1,
                        latestEntry: appendedAsNewest ? entry : current.latestEntry,
                        latestRetryEntry: entry.type === 'StageRetrying'
                            ? entry
                            : current.latestRetryEntry,
                        timelineTypeOptions: nextTimelineTypeOptions,
                        pendingInterviewGates: pendingInterviewMetadata.pendingInterviewGates,
                        hasOlder: resolveHasOlderFromLoadedRange(nextSegments, current.hasOlder),
                        status: current.status === 'idle' ? 'ready' : current.status,
                        error: null,
                        revision,
                        lastMutation: {
                            kind: 'append_live',
                            revision,
                            entry,
                            appendedAsNewest,
                            introducedRetryCorrelation: introducesRetryCorrelation,
                        },
                        _knownSequences: nextKnownSequences,
                        _retryCorrelationEntityKeys: nextRetryCorrelationEntityKeys,
                        _closedInterviewEntityKeys: pendingInterviewMetadata.closedInterviewEntityKeys,
                        _pendingInterviewGateKeys: pendingInterviewMetadata.pendingInterviewGateKeys,
                    },
                },
            }
        }),
    clearRun: (runId) =>
        set((state) => {
            const next = { ...state.byRunId }
            delete next[runId]
            return {
                byRunId: next,
            }
        }),
}))

export function getRunJournalState(runId: string | null): RunJournalStateEntry {
    if (!runId) {
        return createDefaultRunJournalState()
    }
    return resolveRunJournalState(useRunJournalStore.getState().byRunId, runId)
}
