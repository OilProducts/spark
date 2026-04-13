import { useCallback, useMemo, useRef, type SetStateAction } from 'react'
import { ApiHttpError, fetchPipelineAnswerValidated } from '@/lib/attractorClient'
import { useStore } from '@/store'
import type { RunDetailSessionState } from '@/state/viewSessionTypes'
import type {
    GroupedTimelineEntry,
    PendingInterviewGate,
    PendingQuestionSnapshot,
    TimelineEventCategory,
    TimelineSeverity,
} from '../model/shared'
import {
    buildGroupedPendingInterviewGates,
    filterAnsweredPendingInterviewGates,
    logUnexpectedRunError,
    matchesTimelineFilters,
    mergePendingInterviewGatesWithSnapshots,
    timelineCorrelationDescriptorFromEvent,
    toTimelineEvent,
} from '../model/timelineModel'
import { loadSelectedRunJournal } from '../services/runStreamTransport'
import {
    iterateRunJournalEntries,
    useRunJournalStore,
    type RunJournalMutation,
    type RunJournalStateEntry,
} from '../state/runJournalStore'

type UseRunTimelineArgs = {
    pendingQuestionSnapshots: PendingQuestionSnapshot[]
    selectedRunTimelineId: string | null
}

type TimelineProjection = {
    filteredCount: number
    groupedEntries: GroupedTimelineEntry[]
}

const DEFAULT_TIMELINE_SESSION = {
    timelineTypeFilter: 'all',
    timelineNodeStageFilter: '',
    timelineCategoryFilter: 'all' as const,
    timelineSeverityFilter: 'all' as const,
    pendingGateActionError: null as string | null,
    submittingGateIds: {} as Record<string, boolean>,
    answeredGateIds: {} as Record<string, boolean>,
    freeformAnswersByGateId: {} as Record<string, string>,
}

const RUN_JOURNAL_PAGE_SIZE = 100
const DEFAULT_RUN_JOURNAL_STATE: RunJournalStateEntry = {
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

const EMPTY_TIMELINE_PROJECTION: TimelineProjection = {
    filteredCount: 0,
    groupedEntries: [],
}

const buildTimelineProjection = (
    journalState: RunJournalStateEntry,
    filters: {
        timelineTypeFilter: string
        timelineCategoryFilter: 'all' | TimelineEventCategory
        timelineSeverityFilter: 'all' | TimelineSeverity
        timelineNodeStageFilter: string
    },
): TimelineProjection => {
    const groupedEntries: GroupedTimelineEntry[] = []
    const groupedEntryIndex = new Map<string, number>()
    let filteredCount = 0

    for (const event of iterateRunJournalEntries(journalState.segments)) {
        if (!matchesTimelineFilters(event, filters)) {
            continue
        }
        filteredCount += 1
        const correlation = timelineCorrelationDescriptorFromEvent(event, journalState._retryCorrelationEntityKeys)
        if (!correlation) {
            groupedEntries.push({
                id: event.id,
                correlation: null,
                events: [event],
            })
            continue
        }

        const existingIndex = groupedEntryIndex.get(correlation.key)
        if (existingIndex === undefined) {
            groupedEntryIndex.set(correlation.key, groupedEntries.length)
            groupedEntries.push({
                id: `group-${correlation.key}`,
                correlation,
                events: [event],
            })
            continue
        }

        groupedEntries[existingIndex].events.push(event)
    }

    return {
        filteredCount,
        groupedEntries,
    }
}

const applyLiveMutationToProjection = (
    projection: TimelineProjection,
    mutation: Extract<RunJournalMutation, { kind: 'append_live' }>,
    journalState: RunJournalStateEntry,
    filters: {
        timelineTypeFilter: string
        timelineCategoryFilter: 'all' | TimelineEventCategory
        timelineSeverityFilter: 'all' | TimelineSeverity
        timelineNodeStageFilter: string
    },
): TimelineProjection | null => {
    if (!mutation.appendedAsNewest || mutation.introducedRetryCorrelation) {
        return null
    }
    if (!matchesTimelineFilters(mutation.entry, filters)) {
        return projection
    }

    const correlation = timelineCorrelationDescriptorFromEvent(mutation.entry, journalState._retryCorrelationEntityKeys)
    if (!correlation) {
        return {
            filteredCount: projection.filteredCount + 1,
            groupedEntries: [{
                id: mutation.entry.id,
                correlation: null,
                events: [mutation.entry],
            }, ...projection.groupedEntries],
        }
    }

    const existingIndex = projection.groupedEntries.findIndex((entry) => entry.correlation?.key === correlation.key)
    if (existingIndex === -1) {
        return {
            filteredCount: projection.filteredCount + 1,
            groupedEntries: [{
                id: `group-${correlation.key}`,
                correlation,
                events: [mutation.entry],
            }, ...projection.groupedEntries],
        }
    }

    const existingEntry = projection.groupedEntries[existingIndex]
    const nextEntry: GroupedTimelineEntry = {
        ...existingEntry,
        correlation,
        events: [mutation.entry, ...existingEntry.events],
    }
    return {
        filteredCount: projection.filteredCount + 1,
        groupedEntries: [
            nextEntry,
            ...projection.groupedEntries.slice(0, existingIndex),
            ...projection.groupedEntries.slice(existingIndex + 1),
        ],
    }
}

export function useRunTimeline({
    pendingQuestionSnapshots,
    selectedRunTimelineId,
}: UseRunTimelineArgs) {
    const runDetailSessionsByRunId = useStore((state) => state.runDetailSessionsByRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const timelineSession = selectedRunTimelineId
        ? {
            ...DEFAULT_TIMELINE_SESSION,
            ...(runDetailSessionsByRunId[selectedRunTimelineId] ?? {}),
        }
        : DEFAULT_TIMELINE_SESSION
    const journalStateFromStore = useRunJournalStore((state) => (
        selectedRunTimelineId ? state.byRunId[selectedRunTimelineId] : undefined
    ))
    const journalState = journalStateFromStore ?? DEFAULT_RUN_JOURNAL_STATE
    const patchRunJournal = useRunJournalStore((state) => state.patchRun)
    const appendOlderPage = useRunJournalStore((state) => state.appendOlderPage)
    const timelineError = journalState.error || journalState.liveError
    const isTimelineLive = journalState.liveStatus === 'live'
    const timelineFilters = useMemo(() => ({
        timelineTypeFilter: timelineSession.timelineTypeFilter,
        timelineCategoryFilter: timelineSession.timelineCategoryFilter,
        timelineSeverityFilter: timelineSession.timelineSeverityFilter,
        timelineNodeStageFilter: timelineSession.timelineNodeStageFilter,
    }), [
        timelineSession.timelineCategoryFilter,
        timelineSession.timelineNodeStageFilter,
        timelineSession.timelineSeverityFilter,
        timelineSession.timelineTypeFilter,
    ])
    const projectionCacheRef = useRef<{
        filterKey: string
        projection: TimelineProjection
        revision: number
        runId: string | null
    } | null>(null)
    const filterKey = `${timelineFilters.timelineTypeFilter}::${timelineFilters.timelineCategoryFilter}::${timelineFilters.timelineSeverityFilter}::${timelineFilters.timelineNodeStageFilter}`
    const timelineProjection = useMemo(() => {
        if (!selectedRunTimelineId) {
            projectionCacheRef.current = {
                filterKey,
                projection: EMPTY_TIMELINE_PROJECTION,
                revision: journalState.revision,
                runId: null,
            }
            return EMPTY_TIMELINE_PROJECTION
        }

        const previous = projectionCacheRef.current
        const liveMutation = journalState.lastMutation?.kind === 'append_live'
            ? journalState.lastMutation
            : null
        if (
            previous
            && previous.runId === selectedRunTimelineId
            && previous.filterKey === filterKey
            && previous.revision === journalState.revision - 1
            && liveMutation
        ) {
            const nextProjection = applyLiveMutationToProjection(previous.projection, liveMutation, journalState, timelineFilters)
            if (nextProjection) {
                projectionCacheRef.current = {
                    filterKey,
                    projection: nextProjection,
                    revision: journalState.revision,
                    runId: selectedRunTimelineId,
                }
                return nextProjection
            }
        }

        const nextProjection = buildTimelineProjection(journalState, timelineFilters)
        projectionCacheRef.current = {
            filterKey,
            projection: nextProjection,
            revision: journalState.revision,
            runId: selectedRunTimelineId,
        }
        return nextProjection
    }, [filterKey, journalState, journalState.lastMutation, journalState.revision, selectedRunTimelineId, timelineFilters])

    const pendingInterviewGates = useMemo(
        () => mergePendingInterviewGatesWithSnapshots(journalState.pendingInterviewGates, pendingQuestionSnapshots),
        [journalState.pendingInterviewGates, pendingQuestionSnapshots],
    )
    const visiblePendingInterviewGates = useMemo(
        () => filterAnsweredPendingInterviewGates(pendingInterviewGates, timelineSession.answeredGateIds),
        [pendingInterviewGates, timelineSession.answeredGateIds],
    )
    const groupedPendingInterviewGates = useMemo(() => {
        return buildGroupedPendingInterviewGates(visiblePendingInterviewGates)
    }, [visiblePendingInterviewGates])

    const patchTimelineSession = useCallback((patch: Partial<RunDetailSessionState>) => {
        if (!selectedRunTimelineId) {
            return
        }
        updateRunDetailSession(selectedRunTimelineId, patch)
    }, [selectedRunTimelineId, updateRunDetailSession])

    const submitPendingGateAnswer = useCallback(async (gate: PendingInterviewGate, selectedValue: string) => {
        if (!selectedRunTimelineId || !gate.questionId || !selectedValue.trim()) {
            return
        }
        patchTimelineSession({
            pendingGateActionError: null,
            submittingGateIds: {
                ...timelineSession.submittingGateIds,
                [gate.questionId]: true,
            },
        })
        try {
            await fetchPipelineAnswerValidated(selectedRunTimelineId, gate.questionId, selectedValue)
            const nextFreeformAnswers = { ...timelineSession.freeformAnswersByGateId }
            delete nextFreeformAnswers[gate.questionId]
            patchTimelineSession({
                answeredGateIds: {
                    ...timelineSession.answeredGateIds,
                    [gate.questionId]: true,
                },
                freeformAnswersByGateId: nextFreeformAnswers,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            patchTimelineSession({
                pendingGateActionError: err instanceof ApiHttpError
                    ? `Unable to submit answer (HTTP ${err.status})${err.detail ? `: ${err.detail}` : ''}.`
                    : 'Unable to submit answer. Check connection/backend and retry.',
            })
        } finally {
            const nextSubmittingGateIds = { ...timelineSession.submittingGateIds }
            delete nextSubmittingGateIds[gate.questionId]
            patchTimelineSession({
                submittingGateIds: nextSubmittingGateIds,
            })
        }
    }, [patchTimelineSession, selectedRunTimelineId, timelineSession.answeredGateIds, timelineSession.freeformAnswersByGateId, timelineSession.submittingGateIds])

    const loadOlderTimelineEvents = useCallback(async () => {
        if (
            !selectedRunTimelineId
            || journalState.isLoadingOlder
            || !journalState.hasOlder
            || journalState.oldestSequence === null
        ) {
            return
        }
        patchRunJournal(selectedRunTimelineId, {
            isLoadingOlder: true,
            error: null,
        })
        try {
            const page = await loadSelectedRunJournal(selectedRunTimelineId, {
                limit: RUN_JOURNAL_PAGE_SIZE,
                beforeSequence: journalState.oldestSequence,
            })
            appendOlderPage(selectedRunTimelineId, {
                entries: page.entries
                    .map((entry) => toTimelineEvent(entry))
                    .filter((entry): entry is NonNullable<typeof entry> => entry !== null),
                oldestSequence: page.oldest_sequence ?? null,
                newestSequence: page.newest_sequence ?? null,
                hasOlder: page.has_older,
            })
        } catch (error) {
            logUnexpectedRunError(error)
            patchRunJournal(selectedRunTimelineId, {
                isLoadingOlder: false,
                error: error instanceof ApiHttpError
                    ? `Unable to load older journal entries (HTTP ${error.status})${error.detail ? `: ${error.detail}` : ''}.`
                    : 'Unable to load older journal entries. Check connection/backend and retry.',
            })
        }
    }, [
        appendOlderPage,
        journalState.hasOlder,
        journalState.isLoadingOlder,
        journalState.oldestSequence,
        patchRunJournal,
        selectedRunTimelineId,
    ])

    return {
        filteredTimelineEventCount: timelineProjection.filteredCount,
        freeformAnswersByGateId: timelineSession.freeformAnswersByGateId,
        groupedPendingInterviewGates,
        groupedTimelineEntries: timelineProjection.groupedEntries,
        hasOlderTimelineEvents: journalState.hasOlder,
        isTimelineLive,
        isTimelineLoadingOlder: journalState.isLoadingOlder,
        latestRetryTimelineEvent: journalState.latestRetryEntry,
        latestTimelineEvent: journalState.latestEntry,
        loadOlderTimelineEvents,
        pendingGateActionError: timelineSession.pendingGateActionError,
        setFreeformAnswersByGateId: (next: SetStateAction<Record<string, string>>) => patchTimelineSession({
            freeformAnswersByGateId: typeof next === 'function'
                ? next(timelineSession.freeformAnswersByGateId)
                : next,
        }),
        setTimelineCategoryFilter: (value: 'all' | TimelineEventCategory) => patchTimelineSession({ timelineCategoryFilter: value }),
        setTimelineNodeStageFilter: (value: string) => patchTimelineSession({ timelineNodeStageFilter: value }),
        setTimelineSeverityFilter: (value: 'all' | TimelineSeverity) => patchTimelineSession({ timelineSeverityFilter: value }),
        setTimelineTypeFilter: (value: string) => patchTimelineSession({ timelineTypeFilter: value }),
        submittingGateIds: timelineSession.submittingGateIds,
        submitPendingGateAnswer,
        timelineCategoryFilter: timelineSession.timelineCategoryFilter,
        timelineError,
        timelineEventCount: journalState.loadedEntryCount,
        timelineNodeStageFilter: timelineSession.timelineNodeStageFilter,
        timelineSeverityFilter: timelineSession.timelineSeverityFilter,
        timelineTypeFilter: timelineSession.timelineTypeFilter,
        timelineTypeOptions: journalState.timelineTypeOptions,
        visiblePendingInterviewGates,
    }
}
