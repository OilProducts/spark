import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { useCallback, useMemo, type SetStateAction } from 'react'
import { ApiHttpError, fetchPipelineAnswerValidated } from '@/lib/attractorClient'
import { useStore } from '@/store'
import type { RunDetailSessionState } from '@/state/viewSessionTypes'
import type {
    PendingInterviewGate,
    PendingQuestionSnapshot,
    TimelineEventEntry,
} from '../model/shared'
import {
    buildGroupedPendingInterviewGates,
    filterAnsweredPendingInterviewGates,
    logUnexpectedRunError,
    mergePendingInterviewGatesWithSnapshots,
    toTimelineEvent,
} from '../model/timelineModel'
import { loadSelectedRunJournal } from '../services/runStreamTransport'
import {
    iterateRunJournalEntries,
    useRunJournalStore,
    type RunJournalStateEntry,
} from '../state/runJournalStore'

type UseRunTimelineArgs = {
    pendingQuestionSnapshots: PendingQuestionSnapshot[]
    selectedRunTimelineId: string | null
}

const DEFAULT_TIMELINE_SESSION = {
    pendingGateActionError: null as string | null,
    submittingGateIds: {} as Record<string, boolean>,
    answeredGateIds: {} as Record<string, boolean>,
    freeformAnswersByGateId: {} as Record<string, string>,
    gateNotesByGateId: {} as Record<string, string>,
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

export function useRunTimeline({
    pendingQuestionSnapshots,
    selectedRunTimelineId,
}: UseRunTimelineArgs) {
    const runSession = useStore((state) => selectedRunTimelineId ? state.runDetailSessionsByRunId[selectedRunTimelineId] ?? null : null)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const timelineSession = runSession ?? DEFAULT_TIMELINE_SESSION
    const journalStateFromStore = useRunJournalStore((state) => (
        selectedRunTimelineId ? state.byRunId[selectedRunTimelineId] : undefined
    ))
    const journalState = journalStateFromStore ?? DEFAULT_RUN_JOURNAL_STATE
    const patchRunJournal = useRunJournalStore((state) => state.patchRun)
    const appendOlderPage = useRunJournalStore((state) => state.appendOlderPage)
    const timelineError = journalState.error || journalState.liveError
    const isTimelineLive = journalState.liveStatus === 'live'
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
    const latestRunStateTimelineEvent = useMemo<TimelineEventEntry | null>(() => {
        for (const entry of iterateRunJournalEntries(journalState.segments)) {
            if (entry.type !== 'LLMContent') {
                return entry
            }
        }
        return null
    }, [journalState.revision, journalState.segments])

    const patchTimelineSession = useCallback((patch: Partial<RunDetailSessionState>) => {
        if (!selectedRunTimelineId) {
            return
        }
        updateRunDetailSession(selectedRunTimelineId, patch)
    }, [selectedRunTimelineId, updateRunDetailSession])

    const submitPendingGateAnswer = useCallback(async (gate: PendingInterviewGate, selectedValue: string, note?: string) => {
        if (!selectedRunTimelineId || !gate.questionId || !selectedValue.trim()) {
            return
        }
        const session = useStore.getState().runDetailSessionsByRunId[selectedRunTimelineId]
        if (selectSelectedRunId(useStore.getState()) !== selectedRunTimelineId || session?.questionsStatus !== 'ready'
            || !session.pendingQuestionSnapshots.some((question) => question.questionId === gate.questionId)
            || session.submittingGateIds[gate.questionId] || session.answeredGateIds[gate.questionId]) return
        const lifetime = session.lifetime
        const isCurrent = () => useStore.getState().runDetailSessionsByRunId[selectedRunTimelineId]?.lifetime === lifetime
        patchTimelineSession({
            pendingGateActionError: null,
            submittingGateIds: {
                ...session.submittingGateIds,
                [gate.questionId]: true,
            },
        })
        try {
            await fetchPipelineAnswerValidated(gate.runId ?? selectedRunTimelineId, gate.questionId, selectedValue, note)
            if (!isCurrent()) return
            const current = useStore.getState().runDetailSessionsByRunId[selectedRunTimelineId]
            const nextFreeformAnswers = { ...current.freeformAnswersByGateId }
            delete nextFreeformAnswers[gate.questionId]
            const nextGateNotes = { ...current.gateNotesByGateId }
            delete nextGateNotes[gate.questionId]
            patchTimelineSession({
                answeredGateIds: {
                    ...current.answeredGateIds,
                    [gate.questionId]: true,
                },
                freeformAnswersByGateId: nextFreeformAnswers,
                gateNotesByGateId: nextGateNotes,
            })
        } catch (err) {
            if (!isCurrent()) return
            logUnexpectedRunError(err)
            patchTimelineSession({
                pendingGateActionError: err instanceof ApiHttpError
                    ? `Unable to submit answer (HTTP ${err.status})${err.detail ? `: ${err.detail}` : ''}.`
                    : 'Unable to submit answer. Check connection/backend and retry.',
            })
        } finally {
            if (isCurrent()) {
                const nextSubmittingGateIds = { ...useStore.getState().runDetailSessionsByRunId[selectedRunTimelineId].submittingGateIds }
                delete nextSubmittingGateIds[gate.questionId]
                patchTimelineSession({ submittingGateIds: nextSubmittingGateIds })
            }
        }
    }, [patchTimelineSession, selectedRunTimelineId])

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
        confirmedQuestionIds: runSession?.questionsStatus === 'ready' ? runSession.pendingQuestionSnapshots.map((question) => question.questionId) : [],
        freeformAnswersByGateId: timelineSession.freeformAnswersByGateId,
        gateNotesByGateId: timelineSession.gateNotesByGateId,
        groupedPendingInterviewGates,
        hasOlderTimelineEvents: journalState.hasOlder,
        isTimelineLive,
        isTimelineLoadingOlder: journalState.isLoadingOlder,
        latestRetryTimelineEvent: journalState.latestRetryEntry,
        latestTimelineEvent: latestRunStateTimelineEvent,
        loadOlderTimelineEvents,
        pendingGateActionError: timelineSession.pendingGateActionError,
        setFreeformAnswersByGateId: (next: SetStateAction<Record<string, string>>) => patchTimelineSession({
            freeformAnswersByGateId: typeof next === 'function'
                ? next(timelineSession.freeformAnswersByGateId)
                : next,
        }),
        setGateNotesByGateId: (next: SetStateAction<Record<string, string>>) => patchTimelineSession({
            gateNotesByGateId: typeof next === 'function'
                ? next(timelineSession.gateNotesByGateId)
                : next,
        }),
        submittingGateIds: timelineSession.submittingGateIds,
        submitPendingGateAnswer,
        timelineError,
        visiblePendingInterviewGates,
    }
}
