import { useCallback, useMemo, type SetStateAction } from 'react'
import { ApiHttpError, fetchPipelineAnswerValidated } from '@/lib/attractorClient'
import { useStore } from '@/store'
import type { RunDetailSessionState } from '@/state/viewSessionTypes'
import type {
    PendingInterviewGate,
    PendingQuestionSnapshot,
    TimelineEventCategory,
    TimelineSeverity,
} from '../model/shared'
import {
    buildGroupedPendingInterviewGates,
    buildGroupedTimelineEntries,
    buildPendingInterviewGates,
    buildRetryCorrelationEntityKeys,
    buildTimelineTypeOptions,
    filterAnsweredPendingInterviewGates,
    filterTimelineEvents,
    logUnexpectedRunError,
} from '../model/timelineModel'
import { useRunTimelineStream } from './useRunTimelineStream'

type UseRunTimelineArgs = {
    pendingQuestionSnapshots: PendingQuestionSnapshot[]
    selectedRunTimelineId: string | null
    manageSync?: boolean
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

export function useRunTimeline({
    pendingQuestionSnapshots,
    selectedRunTimelineId,
    manageSync = true,
}: UseRunTimelineArgs) {
    const runDetailSessionsByRunId = useStore((state) => state.runDetailSessionsByRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const timelineSession = selectedRunTimelineId
        ? {
            ...DEFAULT_TIMELINE_SESSION,
            ...(runDetailSessionsByRunId[selectedRunTimelineId] ?? {}),
        }
        : DEFAULT_TIMELINE_SESSION
    const {
        isTimelineLive,
        timelineDroppedCount,
        timelineError,
        timelineEvents,
    } = useRunTimelineStream({
        selectedRunTimelineId,
        manageSync,
    })

    const timelineTypeOptions = useMemo(
        () => buildTimelineTypeOptions(timelineEvents),
        [timelineEvents],
    )
    const filteredTimelineEvents = useMemo(() => {
        return filterTimelineEvents(timelineEvents, {
            timelineTypeFilter: timelineSession.timelineTypeFilter,
            timelineCategoryFilter: timelineSession.timelineCategoryFilter,
            timelineSeverityFilter: timelineSession.timelineSeverityFilter,
            timelineNodeStageFilter: timelineSession.timelineNodeStageFilter,
        })
    }, [
        timelineEvents,
        timelineSession.timelineCategoryFilter,
        timelineSession.timelineNodeStageFilter,
        timelineSession.timelineSeverityFilter,
        timelineSession.timelineTypeFilter,
    ])
    const retryCorrelationEntityKeys = useMemo(() => {
        return buildRetryCorrelationEntityKeys(timelineEvents)
    }, [timelineEvents])
    const groupedTimelineEntries = useMemo(() => {
        return buildGroupedTimelineEntries(filteredTimelineEvents, retryCorrelationEntityKeys)
    }, [filteredTimelineEvents, retryCorrelationEntityKeys])

    const pendingInterviewGates = useMemo(() => {
        return buildPendingInterviewGates(timelineEvents, pendingQuestionSnapshots)
    }, [pendingQuestionSnapshots, timelineEvents])
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

    return {
        filteredTimelineEvents,
        freeformAnswersByGateId: timelineSession.freeformAnswersByGateId,
        groupedPendingInterviewGates,
        groupedTimelineEntries,
        isTimelineLive,
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
        timelineDroppedCount,
        timelineError,
        timelineEvents,
        timelineNodeStageFilter: timelineSession.timelineNodeStageFilter,
        timelineSeverityFilter: timelineSession.timelineSeverityFilter,
        timelineTypeFilter: timelineSession.timelineTypeFilter,
        timelineTypeOptions,
        visiblePendingInterviewGates,
    }
}
