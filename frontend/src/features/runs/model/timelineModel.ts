import { ApiHttpError } from '@/lib/attractorClient'
import type {
    PendingInterviewGate,
    PendingInterviewGateGroup,
    PendingQuestionSnapshot,
    PendingQuestionOption,
    GroupedTimelineEntry,
    RunProgressEntry,
    RunProgressProjection,
    TimelineCorrelationDescriptor,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
    TimelineSourceScope,
} from './shared'

const TIMELINE_EVENT_TYPES: Record<string, TimelineEventCategory> = {
    lifecycle: 'lifecycle',
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
    log: 'log',
    runtime: 'runtime',
    LLMContent: 'runtime',
    state: 'state',
    run_meta: 'metadata',
}

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

const timelineSourceScopeFromEvent = (payload: Record<string, unknown>): TimelineSourceScope => (
    payload.source_scope === 'child' ? 'child' : 'root'
)

const timelineSourceParentNodeIdFromEvent = (payload: Record<string, unknown>): string | null => (
    asTrimmedString(payload.source_parent_node_id)
)

const timelineSourceFlowNameFromEvent = (payload: Record<string, unknown>): string | null => (
    asTrimmedString(payload.source_flow_name)
)

const timelineSourcePrefix = (
    sourceScope: TimelineSourceScope,
    sourceParentNodeId: string | null,
    sourceFlowName: string | null,
): string => {
    if (sourceScope !== 'child') {
        return ''
    }
    const parentNodeLabel = sourceParentNodeId || 'parent node'
    if (sourceFlowName) {
        return `Child flow ${sourceFlowName} via ${parentNodeLabel}: `
    }
    return `Child flow via ${parentNodeLabel}: `
}

const timelineSourceLabel = (
    sourceScope: TimelineSourceScope,
    sourceParentNodeId: string | null,
    sourceFlowName: string | null,
): string | null => {
    if (sourceScope !== 'child') {
        return null
    }
    const flowLabel = sourceFlowName ? `Child flow ${sourceFlowName}` : 'Child flow'
    return sourceParentNodeId ? `${flowLabel} via ${sourceParentNodeId}` : flowLabel
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

const timelineSeverityFromJournalEntry = (value: unknown): TimelineSeverity => {
    if (value === 'warning' || value === 'error') {
        return value
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

const timelineSummaryFromEvent = (
    type: string,
    payload: Record<string, unknown>,
    nodeId: string | null,
    sourceScope: TimelineSourceScope,
    sourceParentNodeId: string | null,
    sourceFlowName: string | null,
): string => {
    const sourcePrefix = timelineSourcePrefix(sourceScope, sourceParentNodeId, sourceFlowName)
    if (type === 'PipelineStarted') {
        return `${sourcePrefix}Pipeline started at ${nodeId || 'start'}`
    }
    if (type === 'PipelineCompleted') {
        const outcome = asTrimmedString(payload.outcome)
        const reasonCode = asTrimmedString(payload.outcome_reason_code)
        const reasonMessage = asTrimmedString(payload.outcome_reason_message)
        if (outcome === 'failure') {
            if (reasonMessage) {
                return `${sourcePrefix}Pipeline completed at ${nodeId || 'exit'} (failure: ${reasonMessage})`
            }
            if (reasonCode) {
                return `${sourcePrefix}Pipeline completed at ${nodeId || 'exit'} (failure: ${reasonCode})`
            }
            return `${sourcePrefix}Pipeline completed at ${nodeId || 'exit'} (failure)`
        }
        return outcome
            ? `${sourcePrefix}Pipeline completed at ${nodeId || 'exit'} (${outcome})`
            : `${sourcePrefix}Pipeline completed at ${nodeId || 'exit'}`
    }
    if (type === 'PipelineFailed') {
        const error = typeof payload.error === 'string' && payload.error.trim().length > 0
            ? payload.error.trim()
            : null
        return error ? `${sourcePrefix}Pipeline failed: ${error}` : `${sourcePrefix}Pipeline failed`
    }
    if (type === 'StageStarted') {
        return `${sourcePrefix}Stage ${nodeId || 'unknown'} started`
    }
    if (type === 'StageCompleted') {
        const outcome = typeof payload.outcome === 'string' && payload.outcome.trim().length > 0
            ? payload.outcome.trim()
            : null
        return outcome
            ? `${sourcePrefix}Stage ${nodeId || 'unknown'} completed (${outcome})`
            : `${sourcePrefix}Stage ${nodeId || 'unknown'} completed`
    }
    if (type === 'StageFailed') {
        const error = typeof payload.error === 'string' && payload.error.trim().length > 0
            ? payload.error.trim()
            : null
        return error
            ? `${sourcePrefix}Stage ${nodeId || 'unknown'} failed: ${error}`
            : `${sourcePrefix}Stage ${nodeId || 'unknown'} failed`
    }
    if (type === 'StageRetrying') {
        const attempt = asFiniteNumber(payload.attempt)
        return attempt !== null
            ? `${sourcePrefix}Stage ${nodeId || 'unknown'} retrying (attempt ${attempt})`
            : `${sourcePrefix}Stage ${nodeId || 'unknown'} retrying`
    }
    if (type === 'ParallelStarted') {
        const branchCount = asFiniteNumber(payload.branch_count)
        return branchCount !== null
            ? `${sourcePrefix}Parallel fan-out started (${branchCount} branches)`
            : `${sourcePrefix}Parallel fan-out started`
    }
    if (type === 'ParallelBranchStarted') {
        const branchName = typeof payload.branch === 'string' && payload.branch.trim().length > 0
            ? payload.branch.trim()
            : nodeId || 'unknown'
        return `${sourcePrefix}Parallel branch ${branchName} started`
    }
    if (type === 'ParallelBranchCompleted') {
        const branchName = typeof payload.branch === 'string' && payload.branch.trim().length > 0
            ? payload.branch.trim()
            : nodeId || 'unknown'
        const success = payload.success === true ? 'success' : payload.success === false ? 'failed' : null
        return success
            ? `${sourcePrefix}Parallel branch ${branchName} completed (${success})`
            : `${sourcePrefix}Parallel branch ${branchName} completed`
    }
    if (type === 'ParallelCompleted') {
        const successCount = asFiniteNumber(payload.success_count)
        const failureCount = asFiniteNumber(payload.failure_count)
        if (successCount !== null && failureCount !== null) {
            return `${sourcePrefix}Parallel fan-out completed (${successCount} success, ${failureCount} failed)`
        }
        return `${sourcePrefix}Parallel fan-out completed`
    }
    if (type === 'InterviewStarted') {
        return `${sourcePrefix}Interview started for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewInform') {
        const message = asTrimmedString(payload.message)
            ?? asTrimmedString(payload.prompt)
            ?? asTrimmedString(payload.question)
        return message
            ? `${sourcePrefix}Interview info for ${nodeId || 'human gate'}: ${message}`
            : `${sourcePrefix}Interview info for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewCompleted') {
        const answer = asTrimmedString(payload.answer)
        const provenance = interviewOutcomeProvenanceFromPayload(type, payload)
        if (provenance === 'skipped') {
            return `${sourcePrefix}Interview completed for ${nodeId || 'human gate'} (skipped)`
        }
        if (provenance === 'accepted') {
            return answer
                ? `${sourcePrefix}Interview completed for ${nodeId || 'human gate'} (accepted answer: ${answer})`
                : `${sourcePrefix}Interview completed for ${nodeId || 'human gate'} (accepted answer)`
        }
        return answer
            ? `${sourcePrefix}Interview completed for ${nodeId || 'human gate'} (${answer})`
            : `${sourcePrefix}Interview completed for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewTimeout') {
        const provenance = interviewOutcomeProvenanceFromPayload(type, payload)
        if (provenance === 'timeout_default_applied') {
            const defaultChoiceLabel = interviewDefaultChoiceLabelFromPayload(payload)
            return defaultChoiceLabel
                ? `${sourcePrefix}Interview timed out for ${nodeId || 'human gate'} (default applied: ${defaultChoiceLabel})`
                : `${sourcePrefix}Interview timed out for ${nodeId || 'human gate'} (default applied)`
        }
        if (provenance === 'timeout_no_default') {
            return `${sourcePrefix}Interview timed out for ${nodeId || 'human gate'} (no default applied)`
        }
        return `${sourcePrefix}Interview timed out for ${nodeId || 'human gate'}`
    }
    if (type === 'human_gate') {
        const prompt = typeof payload.prompt === 'string' && payload.prompt.trim().length > 0
            ? payload.prompt.trim()
            : null
        return prompt
            ? `${sourcePrefix}Human gate pending: ${prompt}`
            : `${sourcePrefix}Human gate pending for ${nodeId || 'unknown'}`
    }
    if (type === 'CheckpointSaved') {
        return `${sourcePrefix}Checkpoint saved at ${nodeId || 'current node'}`
    }
    return `${sourcePrefix}${type}`
}

const toTimelineEvent = (value: unknown): TimelineEventEntry | null => {
    const payload = asRecord(value)
    if (!payload) {
        return null
    }
    const journalRawType = asTrimmedString(payload.raw_type)
    const journalKind = asTrimmedString(payload.kind)
    if (journalRawType && journalKind) {
        const sequence = asFiniteNumber(payload.sequence)
        const emittedAt = asTrimmedString(payload.emitted_at)
        const summary = asTrimmedString(payload.summary)
        const journalPayload = asRecord(payload.payload)
        if (sequence === null || emittedAt === null || summary === null || journalPayload === null) {
            return null
        }
        const category = TIMELINE_EVENT_TYPES[journalKind] ?? TIMELINE_EVENT_TYPES[journalRawType] ?? 'other'
        return {
            id: typeof payload.id === 'string' ? payload.id : `event-${sequence}`,
            sequence,
            type: journalRawType,
            category,
            severity: timelineSeverityFromJournalEntry(payload.severity),
            nodeId: asTrimmedString(payload.node_id),
            stageIndex: asFiniteNumber(payload.stage_index),
            summary,
            receivedAt: emittedAt,
            sourceScope: payload.source_scope === 'child' ? 'child' : 'root',
            sourceParentNodeId: asTrimmedString(payload.source_parent_node_id),
            sourceFlowName: asTrimmedString(payload.source_flow_name),
            questionId: asTrimmedString(payload.question_id),
            payload: journalPayload,
        }
    }
    const type = typeof payload.type === 'string' ? payload.type : ''
    const category = TIMELINE_EVENT_TYPES[type]
    if (!category) {
        return null
    }
    const nodeId = timelineNodeIdFromEvent(payload)
    const stageIndex = asFiniteNumber(payload.index)
    const sequence = asFiniteNumber(payload.sequence)
    const emittedAt = asTrimmedString(payload.emitted_at)
    if (sequence === null || emittedAt === null) {
        return null
    }
    const sourceScope = timelineSourceScopeFromEvent(payload)
    const sourceParentNodeId = timelineSourceParentNodeIdFromEvent(payload)
    const sourceFlowName = timelineSourceFlowNameFromEvent(payload)
    return {
        id: `event-${sequence}`,
        sequence,
        type,
        category,
        severity: timelineSeverityFromEvent(type, payload),
        nodeId,
        stageIndex,
        summary: timelineSummaryFromEvent(type, payload, nodeId, sourceScope, sourceParentNodeId, sourceFlowName),
        receivedAt: emittedAt,
        sourceScope,
        sourceParentNodeId,
        sourceFlowName,
        questionId: asTrimmedString(payload.question_id),
        payload,
    }
}

const timelineEntityKey = (event: TimelineEventEntry): string | null => {
    if (!event.nodeId && event.stageIndex === null) {
        return null
    }
    return [
        event.sourceScope,
        event.sourceParentNodeId ?? 'root',
        event.nodeId ?? 'unknown',
        event.stageIndex !== null ? String(event.stageIndex) : 'na',
    ].join('::')
}

const timelineSubjectLabel = (event: TimelineEventEntry): string => {
    if (event.sourceScope !== 'child') {
        return event.nodeId ?? 'unknown'
    }
    const childLabel = event.nodeId ?? 'unknown'
    const sourceLabel = timelineSourceLabel(event.sourceScope, event.sourceParentNodeId, event.sourceFlowName)
    return sourceLabel ? `${childLabel} (${sourceLabel})` : `Child ${childLabel}`
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
    const subjectLabel = timelineSubjectLabel(event)

    if (event.category === 'interview') {
        return {
            key: `interview:${entityKey}`,
            kind: 'interview',
            label: `Interview sequence for ${subjectLabel}${stageSuffix}`,
        }
    }

    if (event.category === 'stage' && RETRY_CORRELATION_EVENT_TYPES.has(event.type) && retryEntityKeys.has(entityKey)) {
        return {
            key: `retry:${entityKey}`,
            kind: 'retry',
            label: `Retry sequence for ${subjectLabel}${stageSuffix}`,
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

type TimelineFilters = {
    timelineTypeFilter: string
    timelineCategoryFilter: 'all' | TimelineEventCategory
    timelineSeverityFilter: 'all' | TimelineSeverity
    timelineNodeStageFilter: string
}

const buildTimelineTypeOptions = (timelineEvents: Iterable<TimelineEventEntry>) => {
    const eventTypes = new Set<string>()
    for (const event of timelineEvents) {
        eventTypes.add(event.type)
    }
    return Array.from(eventTypes).sort((left, right) => left.localeCompare(right))
}

const matchesTimelineFilters = (
    event: TimelineEventEntry,
    {
        timelineTypeFilter,
        timelineCategoryFilter,
        timelineSeverityFilter,
        timelineNodeStageFilter,
    }: TimelineFilters,
) => {
    const normalizedNodeStageFilter = timelineNodeStageFilter.trim().toLowerCase()

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
    const sourceParentNodeMatch = (event.sourceParentNodeId ?? '').toLowerCase().includes(normalizedNodeStageFilter)
    const sourceFlowMatch = (event.sourceFlowName ?? '').toLowerCase().includes(normalizedNodeStageFilter)
    const sourceScopeMatch = event.sourceScope.toLowerCase().includes(normalizedNodeStageFilter)
    return nodeIdMatch || stageIndexMatch || sourceParentNodeMatch || sourceFlowMatch || sourceScopeMatch
}

const filterTimelineEvents = (
    timelineEvents: Iterable<TimelineEventEntry>,
    filters: TimelineFilters,
) => {
    const filteredEvents: TimelineEventEntry[] = []
    for (const event of timelineEvents) {
        if (matchesTimelineFilters(event, filters)) {
            filteredEvents.push(event)
        }
    }
    return filteredEvents
}

const buildRetryCorrelationEntityKeys = (timelineEvents: Iterable<TimelineEventEntry>) => {
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
}

const buildGroupedTimelineEntries = (
    filteredTimelineEvents: Iterable<TimelineEventEntry>,
    retryCorrelationEntityKeys: ReadonlySet<string>,
): GroupedTimelineEntry[] => {
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
}

const pendingInterviewGateDedupeKey = (
    sourceScope: TimelineSourceScope,
    sourceParentNodeId: string | null,
    sourceFlowName: string | null,
    nodeId: string | null,
    prompt: string,
) => (
    [
        sourceScope,
        sourceParentNodeId ?? '',
        sourceFlowName ?? '',
        nodeId ?? '',
        prompt.toLowerCase(),
    ].join('::')
)

const toPendingInterviewGateFromEvent = (event: TimelineEventEntry): PendingInterviewGate | null => {
    if (event.category !== 'interview') {
        return null
    }
    if (event.type !== 'InterviewStarted' && event.type !== 'human_gate' && event.type !== 'InterviewInform') {
        return null
    }

    const questionIdValue = event.questionId ?? event.payload.question_id
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

    return {
        eventId: event.id,
        sequence: event.sequence,
        receivedAt: event.receivedAt,
        nodeId: event.nodeId,
        stageIndex: event.stageIndex,
        sourceScope: event.sourceScope,
        sourceParentNodeId: event.sourceParentNodeId,
        sourceFlowName: event.sourceFlowName,
        prompt,
        questionId,
        questionType,
        options,
    }
}

const buildJournalPendingInterviewGates = (
    timelineEvents: Iterable<TimelineEventEntry>,
) => {
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

        const gate = toPendingInterviewGateFromEvent(event)
        if (!gate) {
            continue
        }
        const dedupeKey = pendingInterviewGateDedupeKey(
            gate.sourceScope,
            gate.sourceParentNodeId,
            gate.sourceFlowName,
            gate.nodeId,
            gate.prompt,
        )
        if (pendingGateKeys.has(dedupeKey)) {
            continue
        }
        pendingGateKeys.add(dedupeKey)
        pendingGates.push(gate)
    }

    return {
        closedEntityKeys,
        pendingGates,
        pendingGateKeys,
    }
}

const mergePendingInterviewGatesWithSnapshots = (
    journalPendingGates: PendingInterviewGate[],
    pendingQuestionSnapshots: PendingQuestionSnapshot[],
) => {
    const pendingGates = [...journalPendingGates]
    const pendingGateKeys = new Set(
        journalPendingGates.map((gate) => pendingInterviewGateDedupeKey(
            gate.sourceScope,
            gate.sourceParentNodeId,
            gate.sourceFlowName,
            gate.nodeId,
            gate.prompt,
        )),
    )

    let nextSequence = pendingGates.reduce((maxSequence, gate) => Math.max(maxSequence, gate.sequence), 0) + 1
    for (const question of pendingQuestionSnapshots) {
        const questionIdMatch = pendingGates.some((gate) => gate.questionId === question.questionId)
        if (questionIdMatch) {
            continue
        }
        const dedupeKey = pendingInterviewGateDedupeKey('root', null, null, question.nodeId, question.prompt)
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
            sourceScope: 'root',
            sourceParentNodeId: null,
            sourceFlowName: null,
            prompt: question.prompt,
            questionId: question.questionId,
            questionType: question.questionType,
            options: question.options,
        })
        nextSequence += 1
    }

    return pendingGates
}

const buildPendingInterviewGates = (
    timelineEvents: Iterable<TimelineEventEntry>,
    pendingQuestionSnapshots: PendingQuestionSnapshot[],
) => mergePendingInterviewGatesWithSnapshots(
    buildJournalPendingInterviewGates(timelineEvents).pendingGates,
    pendingQuestionSnapshots,
)

const filterAnsweredPendingInterviewGates = (
    pendingInterviewGates: PendingInterviewGate[],
    answeredGateIds: Record<string, boolean>,
) => pendingInterviewGates.filter((gate) => !gate.questionId || !answeredGateIds[gate.questionId])

const buildGroupedPendingInterviewGates = (
    visiblePendingInterviewGates: PendingInterviewGate[],
): PendingInterviewGateGroup[] => {
    const grouped = new Map<string, PendingInterviewGateGroup>()
    for (const gate of visiblePendingInterviewGates) {
        const key = [
            gate.sourceScope,
            gate.sourceParentNodeId ?? 'root',
            gate.sourceFlowName ?? '',
            gate.nodeId ?? 'human-gate',
            gate.stageIndex !== null ? String(gate.stageIndex) : 'na',
        ].join('::')
        if (!grouped.has(key)) {
            const headingNode = gate.nodeId ?? 'human gate'
            const headingStage = gate.stageIndex !== null ? ` (index ${gate.stageIndex})` : ''
            const sourceLabel = timelineSourceLabel(gate.sourceScope, gate.sourceParentNodeId, gate.sourceFlowName)
            grouped.set(key, {
                key,
                heading: sourceLabel
                    ? `${headingNode}${headingStage} · ${sourceLabel}`
                    : `${headingNode}${headingStage}`,
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
}

const asProgressChannel = (value: unknown): RunProgressEntry['channel'] | null => {
    if (value === 'assistant' || value === 'reasoning' || value === 'plan') {
        return value
    }
    return null
}

const asProgressStatus = (value: unknown): RunProgressEntry['status'] => (
    value === 'complete' ? 'complete' : 'streaming'
)

const progressSourceKey = (payload: Record<string, unknown>): string => {
    const source = asRecord(payload.source)
    if (!source) {
        return ''
    }
    return [
        asTrimmedString(source.backend) ?? '',
        asTrimmedString(source.app_turn_id) ?? '',
        asTrimmedString(source.item_id) ?? '',
        asTrimmedString(source.response_id) ?? '',
        typeof source.summary_index === 'number' ? String(source.summary_index) : '',
    ].join(':')
}

const buildAllRunProgressEntries = (
    timelineEntries: Iterable<TimelineEventEntry>,
): RunProgressEntry[] => {
    const byKey = new Map<string, RunProgressEntry>()
    const sortedEntries = Array.from(timelineEntries)
        .filter((entry) => entry.type === 'LLMContent')
        .sort((left, right) => left.sequence - right.sequence)

    for (const entry of sortedEntries) {
        const payload = entry.payload
        const channel = asProgressChannel(payload.channel)
        const contentDelta = typeof payload.content_delta === 'string' ? payload.content_delta : ''
        if (!channel || !contentDelta) {
            continue
        }
        const nodeId = entry.nodeId ?? asTrimmedString(payload.node_id)
        const key = [
            entry.sourceScope,
            entry.sourceParentNodeId ?? 'root',
            nodeId ?? 'run',
            channel,
            progressSourceKey(payload),
        ].join('::')
        const previous = byKey.get(key)
        const status = asProgressStatus(payload.status)
        byKey.set(key, {
            id: `progress-${key}`,
            nodeId,
            channel,
            status,
            content: status === 'complete'
                ? contentDelta
                : `${previous?.content ?? ''}${contentDelta}`,
            updatedAt: entry.receivedAt,
            latestSequence: entry.sequence,
        })
    }

    return Array.from(byKey.values())
        .sort((left, right) => right.latestSequence - left.latestSequence)
}

const buildRunProgressEntries = (
    timelineEntries: Iterable<TimelineEventEntry>,
    limit = 5,
): RunProgressEntry[] => (
    buildAllRunProgressEntries(timelineEntries)
        .slice(0, Math.max(1, limit))
)

const buildRunProgressProjection = (
    timelineEntries: Iterable<TimelineEventEntry>,
    currentNodeId: string | null | undefined,
    limit = 5,
): RunProgressProjection => {
    const allEntries = buildAllRunProgressEntries(timelineEntries)
    const normalizedCurrentNodeId = currentNodeId?.trim() || null
    const activeEntry = normalizedCurrentNodeId
        ? allEntries.find((entry) => entry.nodeId === normalizedCurrentNodeId) ?? null
        : null
    const nodeOptions: string[] = []
    const seenNodeOptions = new Set<string>()
    for (const entry of allEntries) {
        if (!entry.nodeId || seenNodeOptions.has(entry.nodeId)) {
            continue
        }
        seenNodeOptions.add(entry.nodeId)
        nodeOptions.push(entry.nodeId)
    }

    return {
        activeEntry,
        recentEntries: allEntries
            .filter((entry) => entry.id !== activeEntry?.id)
            .slice(0, Math.max(1, limit)),
        nodeOptions,
    }
}

export {
    buildRunProgressProjection,
    buildRunProgressEntries,
    buildGroupedPendingInterviewGates,
    buildGroupedTimelineEntries,
    buildJournalPendingInterviewGates,
    buildPendingInterviewGates,
    buildRetryCorrelationEntityKeys,
    buildTimelineTypeOptions,
    filterAnsweredPendingInterviewGates,
    filterTimelineEvents,
    matchesTimelineFilters,
    mergePendingInterviewGatesWithSnapshots,
    PENDING_GATE_FALLBACK_RECEIVED_AT,
    asFiniteNumber,
    logUnexpectedRunError,
    pendingGateOptionsFromPayload,
    pendingInterviewGateDedupeKey,
    pendingGateQuestionTypeFromPayload,
    pendingGateSemanticFallbackOptions,
    timelineCorrelationDescriptorFromEvent,
    timelineEntityKey,
    toPendingInterviewGateFromEvent,
    toTimelineEvent,
}
