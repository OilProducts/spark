import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ApiHttpError, fetchPipelineAnswerValidated, pipelineEventsUrl } from '@/lib/attractorClient'
import type {
    GroupedTimelineEntry,
    PendingInterviewGate,
    PendingInterviewGateGroup,
    PendingQuestionSnapshot,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
} from '../model/shared'
import {
    TIMELINE_MAX_ITEMS,
    PENDING_GATE_FALLBACK_RECEIVED_AT,
    asFiniteNumber,
    logUnexpectedRunError,
    pendingGateOptionsFromPayload,
    pendingGateQuestionTypeFromPayload,
    pendingGateSemanticFallbackOptions,
    timelineCorrelationDescriptorFromEvent,
    timelineEntityKey,
    toTimelineEvent,
} from '../model/timelineModel'

type UseRunTimelineArgs = {
    pendingQuestionSnapshots: PendingQuestionSnapshot[]
    selectedRunTimelineId: string | null
    viewMode: string
}

export function useRunTimeline({
    pendingQuestionSnapshots,
    selectedRunTimelineId,
    viewMode,
}: UseRunTimelineArgs) {
    const [timelineEvents, setTimelineEvents] = useState<TimelineEventEntry[]>([])
    const [timelineError, setTimelineError] = useState<string | null>(null)
    const [isTimelineLive, setIsTimelineLive] = useState(false)
    const [timelineTypeFilter, setTimelineTypeFilter] = useState('all')
    const [timelineNodeStageFilter, setTimelineNodeStageFilter] = useState('')
    const [timelineCategoryFilter, setTimelineCategoryFilter] = useState<'all' | TimelineEventCategory>('all')
    const [timelineSeverityFilter, setTimelineSeverityFilter] = useState<'all' | TimelineSeverity>('all')
    const [pendingGateActionError, setPendingGateActionError] = useState<string | null>(null)
    const [submittingGateIds, setSubmittingGateIds] = useState<Record<string, boolean>>({})
    const [answeredGateIds, setAnsweredGateIds] = useState<Record<string, boolean>>({})
    const [freeformAnswersByGateId, setFreeformAnswersByGateId] = useState<Record<string, string>>({})
    const timelineSequenceRef = useRef(0)

    useEffect(() => {
        setPendingGateActionError(null)
        setSubmittingGateIds({})
        setAnsweredGateIds({})
        setFreeformAnswersByGateId({})
    }, [selectedRunTimelineId])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunTimelineId) {
            timelineSequenceRef.current = 0
            setTimelineEvents([])
            setTimelineError(null)
            setIsTimelineLive(false)
            setTimelineTypeFilter('all')
            setTimelineNodeStageFilter('')
            setTimelineCategoryFilter('all')
            setTimelineSeverityFilter('all')
            return
        }

        timelineSequenceRef.current = 0
        setTimelineEvents([])
        setTimelineError(null)
        setIsTimelineLive(false)
        setTimelineTypeFilter('all')
        setTimelineNodeStageFilter('')
        setTimelineCategoryFilter('all')
        setTimelineSeverityFilter('all')

        const source = new EventSource(pipelineEventsUrl(selectedRunTimelineId))
        source.onopen = () => {
            setTimelineError(null)
            setIsTimelineLive(true)
        }
        source.onmessage = (event) => {
            try {
                const payload = JSON.parse(event.data) as unknown
                const sequence = timelineSequenceRef.current
                const timelineEvent = toTimelineEvent(payload, sequence)
                if (!timelineEvent) {
                    return
                }
                timelineSequenceRef.current = sequence + 1
                setTimelineEvents((current) => [timelineEvent, ...current].slice(0, TIMELINE_MAX_ITEMS))
            } catch {
                // ignore malformed events
            }
        }
        source.onerror = () => {
            setIsTimelineLive(false)
            setTimelineError((current) => current || 'Event timeline stream unavailable. Reopen this run to retry.')
        }

        return () => {
            source.close()
            setIsTimelineLive(false)
        }
    }, [selectedRunTimelineId, viewMode])

    const timelineTypeOptions = useMemo(
        () => Array.from(new Set(timelineEvents.map((event) => event.type))).sort((left, right) => left.localeCompare(right)),
        [timelineEvents],
    )
    const filteredTimelineEvents = useMemo(() => {
        const normalizedNodeStageFilter = timelineNodeStageFilter.trim().toLowerCase()

        return timelineEvents.filter((event) => {
            if (timelineTypeFilter !== 'all' && event.type !== timelineTypeFilter) {
                return false
            }
            if (timelineCategoryFilter !== 'all' && event.category !== timelineCategoryFilter) {
                return false
            }
            if (timelineSeverityFilter !== 'all' && event.severity !== timelineSeverityFilter) {
                return false
            }
            if (!normalizedNodeStageFilter) {
                return true
            }

            const nodeIdMatch = (event.nodeId ?? '').toLowerCase().includes(normalizedNodeStageFilter)
            const stageIndexMatch = event.stageIndex !== null && String(event.stageIndex).includes(normalizedNodeStageFilter)
            return nodeIdMatch || stageIndexMatch
        })
    }, [timelineCategoryFilter, timelineEvents, timelineNodeStageFilter, timelineSeverityFilter, timelineTypeFilter])
    const retryCorrelationEntityKeys = useMemo(() => {
        const keys = new Set<string>()
        for (const event of timelineEvents) {
            const entityKey = timelineEntityKey(event)
            if (!entityKey) {
                continue
            }
            if (event.type === 'StageRetrying' || asFiniteNumber(event.payload.attempt) !== null) {
                keys.add(entityKey)
            }
        }
        return keys
    }, [timelineEvents])
    const groupedTimelineEntries = useMemo(() => {
        const entries: GroupedTimelineEntry[] = []
        const groupedEntryIndex = new Map<string, number>()

        for (const event of filteredTimelineEvents) {
            const correlation = timelineCorrelationDescriptorFromEvent(event, retryCorrelationEntityKeys)
            if (!correlation) {
                entries.push({
                    id: event.id,
                    correlation: null,
                    events: [event],
                })
                continue
            }

            const existingIndex = groupedEntryIndex.get(correlation.key)
            if (existingIndex === undefined) {
                groupedEntryIndex.set(correlation.key, entries.length)
                entries.push({
                    id: `group-${correlation.key}`,
                    correlation,
                    events: [event],
                })
                continue
            }

            entries[existingIndex].events.push(event)
        }

        return entries
    }, [filteredTimelineEvents, retryCorrelationEntityKeys])
    const timelineDroppedCount = Math.max(0, timelineSequenceRef.current - timelineEvents.length)

    const pendingInterviewGates = useMemo(() => {
        const closedEntityKeys = new Set<string>()
        const pendingGates: PendingInterviewGate[] = []
        const pendingGateKeys = new Set<string>()
        for (const event of timelineEvents) {
            if (event.category !== 'interview') {
                continue
            }
            const entityKey = timelineEntityKey(event) || `event:${event.id}`
            if (closedEntityKeys.has(entityKey)) {
                continue
            }
            if (event.type === 'InterviewCompleted' || event.type === 'InterviewTimeout') {
                closedEntityKeys.add(entityKey)
                continue
            }
            if (event.type !== 'InterviewStarted' && event.type !== 'human_gate' && event.type !== 'InterviewInform') {
                continue
            }

            const questionIdValue = event.payload.question_id
            const questionId = typeof questionIdValue === 'string' && questionIdValue.trim().length > 0
                ? questionIdValue.trim()
                : null
            const questionType = pendingGateQuestionTypeFromPayload(event.payload)
            const payloadOptions = pendingGateOptionsFromPayload(event.payload)
            const options = payloadOptions.length > 0
                ? payloadOptions
                : pendingGateSemanticFallbackOptions(questionType)
            const questionPrompt = event.payload.question
            const gatePrompt = event.payload.prompt
            const informMessage = event.payload.message
            const prompt = typeof questionPrompt === 'string' && questionPrompt.trim().length > 0
                ? questionPrompt.trim()
                : typeof gatePrompt === 'string' && gatePrompt.trim().length > 0
                    ? gatePrompt.trim()
                    : typeof informMessage === 'string' && informMessage.trim().length > 0
                        ? informMessage.trim()
                        : event.summary
            const dedupeKey = `${event.nodeId ?? ''}::${prompt.toLowerCase()}`
            if (pendingGateKeys.has(dedupeKey)) {
                continue
            }
            pendingGateKeys.add(dedupeKey)
            pendingGates.push({
                eventId: event.id,
                sequence: event.sequence,
                receivedAt: event.receivedAt,
                nodeId: event.nodeId,
                stageIndex: event.stageIndex,
                prompt,
                questionId,
                questionType,
                options,
            })
        }
        let nextSequence = pendingGates.reduce((maxSequence, gate) => Math.max(maxSequence, gate.sequence), 0) + 1
        for (const question of pendingQuestionSnapshots) {
            const questionIdMatch = pendingGates.some((gate) => gate.questionId === question.questionId)
            if (questionIdMatch) {
                continue
            }
            const dedupeKey = `${question.nodeId ?? ''}::${question.prompt.toLowerCase()}`
            if (pendingGateKeys.has(dedupeKey)) {
                continue
            }
            pendingGateKeys.add(dedupeKey)
            pendingGates.push({
                eventId: `question:${question.questionId}`,
                sequence: nextSequence,
                receivedAt: PENDING_GATE_FALLBACK_RECEIVED_AT,
                nodeId: question.nodeId,
                stageIndex: null,
                prompt: question.prompt,
                questionId: question.questionId,
                questionType: question.questionType,
                options: question.options,
            })
            nextSequence += 1
        }
        return pendingGates
    }, [pendingQuestionSnapshots, timelineEvents])
    const visiblePendingInterviewGates = useMemo(
        () => pendingInterviewGates.filter((gate) => !gate.questionId || !answeredGateIds[gate.questionId]),
        [answeredGateIds, pendingInterviewGates],
    )
    const groupedPendingInterviewGates = useMemo(() => {
        const grouped = new Map<string, PendingInterviewGateGroup>()
        for (const gate of visiblePendingInterviewGates) {
            const key = `${gate.nodeId ?? 'human-gate'}::${gate.stageIndex !== null ? String(gate.stageIndex) : 'na'}`
            if (!grouped.has(key)) {
                const headingNode = gate.nodeId ?? 'human gate'
                const headingStage = gate.stageIndex !== null ? ` (index ${gate.stageIndex})` : ''
                grouped.set(key, {
                    key,
                    heading: `${headingNode}${headingStage}`,
                    gates: [],
                })
            }
            grouped.get(key)?.gates.push(gate)
        }
        const sortedGroups = Array.from(grouped.values()).map((group) => ({
            ...group,
            gates: [...group.gates].sort((left, right) => left.sequence - right.sequence),
        }))
        sortedGroups.sort((left, right) => {
            const leftSequence = left.gates[0]?.sequence ?? Number.MAX_SAFE_INTEGER
            const rightSequence = right.gates[0]?.sequence ?? Number.MAX_SAFE_INTEGER
            if (leftSequence !== rightSequence) {
                return leftSequence - rightSequence
            }
            return left.key.localeCompare(right.key)
        })
        return sortedGroups
    }, [visiblePendingInterviewGates])

    const submitPendingGateAnswer = useCallback(async (gate: PendingInterviewGate, selectedValue: string) => {
        if (!selectedRunTimelineId || !gate.questionId || !selectedValue.trim()) {
            return
        }
        setPendingGateActionError(null)
        setSubmittingGateIds((previous) => ({
            ...previous,
            [gate.questionId!]: true,
        }))
        try {
            await fetchPipelineAnswerValidated(selectedRunTimelineId, gate.questionId, selectedValue)
            setAnsweredGateIds((previous) => ({
                ...previous,
                [gate.questionId!]: true,
            }))
            setFreeformAnswersByGateId((previous) => {
                const next = { ...previous }
                delete next[gate.questionId!]
                return next
            })
        } catch (err) {
            logUnexpectedRunError(err)
            if (err instanceof ApiHttpError) {
                const detailSuffix = err.detail ? `: ${err.detail}` : ''
                setPendingGateActionError(`Unable to submit answer (HTTP ${err.status})${detailSuffix}.`)
            } else {
                setPendingGateActionError('Unable to submit answer. Check connection/backend and retry.')
            }
        } finally {
            setSubmittingGateIds((previous) => {
                const next = { ...previous }
                delete next[gate.questionId!]
                return next
            })
        }
    }, [selectedRunTimelineId])

    return {
        filteredTimelineEvents,
        freeformAnswersByGateId,
        groupedPendingInterviewGates,
        groupedTimelineEntries,
        isTimelineLive,
        pendingGateActionError,
        setFreeformAnswersByGateId,
        setTimelineCategoryFilter,
        setTimelineNodeStageFilter,
        setTimelineSeverityFilter,
        setTimelineTypeFilter,
        submittingGateIds,
        submitPendingGateAnswer,
        timelineCategoryFilter,
        timelineDroppedCount,
        timelineError,
        timelineEvents,
        timelineNodeStageFilter,
        timelineSeverityFilter,
        timelineTypeFilter,
        timelineTypeOptions,
        visiblePendingInterviewGates,
    }
}
