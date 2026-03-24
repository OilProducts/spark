import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ApiHttpError, fetchPipelineAnswerValidated, pipelineEventsUrl } from '@/lib/attractorClient'
import type {
    GroupedTimelineEntry,
    PendingInterviewGate,
    PendingInterviewGateGroup,
    PendingQuestionOption,
    PendingQuestionSnapshot,
    TimelineCorrelationDescriptor,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
} from '../shared'

const TIMELINE_EVENT_TYPES: Record<string, TimelineEventCategory> = {
    PipelineStarted: 'lifecycle',
    PipelineCompleted: 'lifecycle',
    PipelineFailed: 'lifecycle',
    StageStarted: 'stage',
    StageCompleted: 'stage',
    StageFailed: 'stage',
    StageRetrying: 'stage',
    ParallelStarted: 'parallel',
    ParallelBranchStarted: 'parallel',
    ParallelBranchCompleted: 'parallel',
    ParallelCompleted: 'parallel',
    InterviewStarted: 'interview',
    InterviewInform: 'interview',
    InterviewCompleted: 'interview',
    InterviewTimeout: 'interview',
    human_gate: 'interview',
    CheckpointSaved: 'checkpoint',
}

const TIMELINE_MAX_ITEMS = 200
const RETRY_CORRELATION_EVENT_TYPES = new Set(['StageStarted', 'StageFailed', 'StageRetrying', 'StageCompleted'])
const PENDING_GATE_FALLBACK_RECEIVED_AT = '1970-01-01T00:00:00Z'

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

const asRecord = (value: unknown): Record<string, unknown> | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    return value as Record<string, unknown>
}

const asFiniteNumber = (value: unknown): number | null => {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return null
    }
    return value
}

const timelineNodeIdFromEvent = (payload: Record<string, unknown>): string | null => {
    const candidates = [payload.node_id, payload.node, payload.name, payload.stage]
    for (const value of candidates) {
        if (typeof value === 'string' && value.trim().length > 0) {
            return value.trim()
        }
    }
    return null
}

const timelineSeverityFromEvent = (type: string, payload: Record<string, unknown>): TimelineSeverity => {
    const severityValue = typeof payload.severity === 'string'
        ? payload.severity
        : typeof payload.level === 'string'
            ? payload.level
            : ''
    const normalized = severityValue.trim().toLowerCase()
    if (normalized === 'info' || normalized === 'warning' || normalized === 'error') {
        return normalized
    }
    if (type === 'PipelineCompleted') {
        return payload.outcome === 'failure' ? 'warning' : 'info'
    }
    if (type === 'PipelineFailed' || type === 'StageFailed') {
        return 'error'
    }
    if (type === 'StageRetrying' || type === 'InterviewTimeout') {
        return 'warning'
    }
    return 'info'
}

type InterviewOutcomeProvenance = 'accepted' | 'skipped' | 'timeout_default_applied' | 'timeout_no_default' | null

const asTrimmedString = (value: unknown): string | null => {
    if (typeof value !== 'string') {
        return null
    }
    const trimmed = value.trim()
    return trimmed.length > 0 ? trimmed : null
}

const interviewOutcomeProvenanceFromPayload = (
    type: string,
    payload: Record<string, unknown>,
): InterviewOutcomeProvenance => {
    const rawProvenance = asTrimmedString(payload.outcome_provenance) ?? asTrimmedString(payload.provenance)
    const normalizedProvenance = rawProvenance?.toLowerCase()
    if (
        normalizedProvenance === 'accepted'
        || normalizedProvenance === 'skipped'
        || normalizedProvenance === 'timeout_default_applied'
        || normalizedProvenance === 'timeout_no_default'
    ) {
        return normalizedProvenance
    }

    if (type === 'InterviewCompleted') {
        const answer = asTrimmedString(payload.answer)
        if (!answer) {
            return null
        }
        return answer.toLowerCase() === 'skipped' ? 'skipped' : 'accepted'
    }

    if (type === 'InterviewTimeout') {
        const defaultChoice = asTrimmedString(payload.default_choice_label)
            ?? asTrimmedString(payload.default_choice_target)
        return defaultChoice ? 'timeout_default_applied' : 'timeout_no_default'
    }

    return null
}

const interviewDefaultChoiceLabelFromPayload = (payload: Record<string, unknown>): string | null => (
    asTrimmedString(payload.default_choice_label)
    ?? asTrimmedString(payload.default_choice_target)
)

const timelineSummaryFromEvent = (type: string, payload: Record<string, unknown>, nodeId: string | null): string => {
    if (type === 'PipelineStarted') {
        return `Pipeline started at ${nodeId || 'start'}`
    }
    if (type === 'PipelineCompleted') {
        const outcome = asTrimmedString(payload.outcome)
        const reasonCode = asTrimmedString(payload.outcome_reason_code)
        const reasonMessage = asTrimmedString(payload.outcome_reason_message)
        if (outcome === 'failure') {
            if (reasonMessage) {
                return `Pipeline completed at ${nodeId || 'exit'} (failure: ${reasonMessage})`
            }
            if (reasonCode) {
                return `Pipeline completed at ${nodeId || 'exit'} (failure: ${reasonCode})`
            }
            return `Pipeline completed at ${nodeId || 'exit'} (failure)`
        }
        return outcome
            ? `Pipeline completed at ${nodeId || 'exit'} (${outcome})`
            : `Pipeline completed at ${nodeId || 'exit'}`
    }
    if (type === 'PipelineFailed') {
        const error = typeof payload.error === 'string' && payload.error.trim().length > 0
            ? payload.error.trim()
            : null
        return error ? `Pipeline failed: ${error}` : 'Pipeline failed'
    }
    if (type === 'StageStarted') {
        return `Stage ${nodeId || 'unknown'} started`
    }
    if (type === 'StageCompleted') {
        const outcome = typeof payload.outcome === 'string' && payload.outcome.trim().length > 0
            ? payload.outcome.trim()
            : null
        return outcome
            ? `Stage ${nodeId || 'unknown'} completed (${outcome})`
            : `Stage ${nodeId || 'unknown'} completed`
    }
    if (type === 'StageFailed') {
        const error = typeof payload.error === 'string' && payload.error.trim().length > 0
            ? payload.error.trim()
            : null
        return error
            ? `Stage ${nodeId || 'unknown'} failed: ${error}`
            : `Stage ${nodeId || 'unknown'} failed`
    }
    if (type === 'StageRetrying') {
        const attempt = asFiniteNumber(payload.attempt)
        return attempt !== null
            ? `Stage ${nodeId || 'unknown'} retrying (attempt ${attempt})`
            : `Stage ${nodeId || 'unknown'} retrying`
    }
    if (type === 'ParallelStarted') {
        const branchCount = asFiniteNumber(payload.branch_count)
        return branchCount !== null ? `Parallel fan-out started (${branchCount} branches)` : 'Parallel fan-out started'
    }
    if (type === 'ParallelBranchStarted') {
        const branchName = typeof payload.branch === 'string' && payload.branch.trim().length > 0
            ? payload.branch.trim()
            : nodeId || 'unknown'
        return `Parallel branch ${branchName} started`
    }
    if (type === 'ParallelBranchCompleted') {
        const branchName = typeof payload.branch === 'string' && payload.branch.trim().length > 0
            ? payload.branch.trim()
            : nodeId || 'unknown'
        const success = payload.success === true ? 'success' : payload.success === false ? 'failed' : null
        return success
            ? `Parallel branch ${branchName} completed (${success})`
            : `Parallel branch ${branchName} completed`
    }
    if (type === 'ParallelCompleted') {
        const successCount = asFiniteNumber(payload.success_count)
        const failureCount = asFiniteNumber(payload.failure_count)
        if (successCount !== null && failureCount !== null) {
            return `Parallel fan-out completed (${successCount} success, ${failureCount} failed)`
        }
        return 'Parallel fan-out completed'
    }
    if (type === 'InterviewStarted') {
        return `Interview started for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewInform') {
        const message = asTrimmedString(payload.message)
            ?? asTrimmedString(payload.prompt)
            ?? asTrimmedString(payload.question)
        return message
            ? `Interview info for ${nodeId || 'human gate'}: ${message}`
            : `Interview info for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewCompleted') {
        const answer = asTrimmedString(payload.answer)
        const provenance = interviewOutcomeProvenanceFromPayload(type, payload)
        if (provenance === 'skipped') {
            return `Interview completed for ${nodeId || 'human gate'} (skipped)`
        }
        if (provenance === 'accepted') {
            return answer
                ? `Interview completed for ${nodeId || 'human gate'} (accepted answer: ${answer})`
                : `Interview completed for ${nodeId || 'human gate'} (accepted answer)`
        }
        return answer
            ? `Interview completed for ${nodeId || 'human gate'} (${answer})`
            : `Interview completed for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewTimeout') {
        const provenance = interviewOutcomeProvenanceFromPayload(type, payload)
        if (provenance === 'timeout_default_applied') {
            const defaultChoiceLabel = interviewDefaultChoiceLabelFromPayload(payload)
            return defaultChoiceLabel
                ? `Interview timed out for ${nodeId || 'human gate'} (default applied: ${defaultChoiceLabel})`
                : `Interview timed out for ${nodeId || 'human gate'} (default applied)`
        }
        if (provenance === 'timeout_no_default') {
            return `Interview timed out for ${nodeId || 'human gate'} (no default applied)`
        }
        return `Interview timed out for ${nodeId || 'human gate'}`
    }
    if (type === 'human_gate') {
        const prompt = typeof payload.prompt === 'string' && payload.prompt.trim().length > 0
            ? payload.prompt.trim()
            : null
        return prompt
            ? `Human gate pending: ${prompt}`
            : `Human gate pending for ${nodeId || 'unknown'}`
    }
    if (type === 'CheckpointSaved') {
        return `Checkpoint saved at ${nodeId || 'current node'}`
    }
    return type
}

const toTimelineEvent = (value: unknown, sequence: number): TimelineEventEntry | null => {
    const payload = asRecord(value)
    if (!payload) {
        return null
    }
    const type = typeof payload.type === 'string' ? payload.type : ''
    const category = TIMELINE_EVENT_TYPES[type]
    if (!category) {
        return null
    }
    const nodeId = timelineNodeIdFromEvent(payload)
    const stageIndex = asFiniteNumber(payload.index)
    return {
        id: `${type}-${sequence}`,
        sequence,
        type,
        category,
        severity: timelineSeverityFromEvent(type, payload),
        nodeId,
        stageIndex,
        summary: timelineSummaryFromEvent(type, payload, nodeId),
        receivedAt: new Date().toISOString(),
        payload,
    }
}

const timelineEntityKey = (event: TimelineEventEntry): string | null => {
    if (!event.nodeId && event.stageIndex === null) {
        return null
    }
    return `${event.nodeId ?? 'unknown'}::${event.stageIndex !== null ? String(event.stageIndex) : 'na'}`
}

const timelineCorrelationDescriptorFromEvent = (
    event: TimelineEventEntry,
    retryEntityKeys: ReadonlySet<string>,
): TimelineCorrelationDescriptor | null => {
    const entityKey = timelineEntityKey(event)
    if (!entityKey) {
        return null
    }
    const stageSuffix = event.stageIndex !== null ? ` (index ${event.stageIndex})` : ''
    const subject = event.nodeId ?? 'unknown'

    if (event.category === 'interview') {
        return {
            key: `interview:${entityKey}`,
            kind: 'interview',
            label: `Interview sequence for ${subject}${stageSuffix}`,
        }
    }

    if (event.category === 'stage' && RETRY_CORRELATION_EVENT_TYPES.has(event.type) && retryEntityKeys.has(entityKey)) {
        return {
            key: `retry:${entityKey}`,
            kind: 'retry',
            label: `Retry sequence for ${subject}${stageSuffix}`,
        }
    }

    return null
}

const asStringOption = (value: unknown): PendingQuestionOption | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    const candidate = value as Record<string, unknown>
    const rawLabel = typeof candidate.label === 'string' ? candidate.label.trim() : ''
    const rawValue = typeof candidate.value === 'string' ? candidate.value.trim() : ''
    if (!rawLabel || !rawValue) {
        return null
    }
    const rawKey = typeof candidate.key === 'string' ? candidate.key.trim() : ''
    const metadata = asRecord(candidate.metadata)
    const rawMetadataDescription = metadata && typeof metadata.description === 'string'
        ? metadata.description.trim()
        : ''
    const rawDescription = typeof candidate.description === 'string' ? candidate.description.trim() : ''
    return {
        label: rawLabel,
        value: rawValue,
        key: rawKey || null,
        description: rawDescription || rawMetadataDescription || null,
    }
}

const pendingGateOptionsFromPayload = (payload: Record<string, unknown>) => {
    const rawOptions = Array.isArray(payload.options) ? payload.options : []
    const seenValues = new Set<string>()
    const options: PendingQuestionOption[] = []
    for (const rawOption of rawOptions) {
        const option = asStringOption(rawOption)
        if (!option || seenValues.has(option.value)) {
            continue
        }
        seenValues.add(option.value)
        options.push(option)
    }
    return options
}

const pendingGateQuestionTypeFromPayload = (
    payload: Record<string, unknown>,
): PendingInterviewGate['questionType'] => {
    const candidateValue = typeof payload.question_type === 'string'
        ? payload.question_type
        : typeof payload.questionType === 'string'
            ? payload.questionType
            : ''
    const normalized = candidateValue.trim().toUpperCase()
    if (
        normalized === 'MULTIPLE_CHOICE'
        || normalized === 'YES_NO'
        || normalized === 'CONFIRMATION'
        || normalized === 'FREEFORM'
    ) {
        return normalized
    }
    const rawOptions = Array.isArray(payload.options) ? payload.options : []
    return rawOptions.length > 0 ? 'MULTIPLE_CHOICE' : null
}

const pendingGateSemanticFallbackOptions = (
    questionType: PendingInterviewGate['questionType'],
): PendingQuestionOption[] => {
    if (questionType === 'YES_NO') {
        return [
            { label: 'Yes', value: 'YES', key: 'Y', description: null },
            { label: 'No', value: 'NO', key: 'N', description: null },
        ]
    }
    if (questionType === 'CONFIRMATION') {
        return [
            { label: 'Confirm', value: 'YES', key: 'Y', description: null },
            { label: 'Cancel', value: 'NO', key: 'N', description: null },
        ]
    }
    return []
}

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
