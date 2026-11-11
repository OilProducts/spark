import { ApiHttpError } from '@/lib/attractorClient'
import type {
    PendingInterviewGate,
    PendingInterviewGateGroup,
    PendingQuestionSnapshot,
    PendingQuestionOption,
    GroupedTimelineEntry,
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
    StageTransition: 'stage',
    StageRetrying: 'stage',
    ParallelStarted: 'parallel',
    ParallelBranchStarted: 'parallel',
    ParallelBranchCompleted: 'parallel',
    ParallelBranchNodeStarted: 'parallel',
    ParallelBranchNodeCompleted: 'parallel',
    ParallelCompleted: 'parallel',
    InterviewStarted: 'interview',
    InterviewInform: 'interview',
    InterviewCompleted: 'interview',
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
    if (type === 'StageRetrying') {
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

type InterviewOutcomeProvenance = 'accepted' | 'skipped' | null

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
    return null
}

// The journal names steps the way the Steps list does: `nodeLabel` resolves a node id to its
// flow label (ids in a child flow resolve against that flow, so the caller gets the scope).
type NodeLabel = (nodeId: string, scope: TimelineSourceScope) => string

const timelineSummaryFromEvent = (
    type: string,
    payload: Record<string, unknown>,
    nodeId: string | null,
    sourceScope: TimelineSourceScope,
    sourceParentNodeId: string | null,
    sourceFlowName: string | null,
    nodeLabel: NodeLabel = (id) => id,
): string | null => {
    const sourcePrefix = timelineSourcePrefix(
        sourceScope,
        sourceParentNodeId && nodeLabel(sourceParentNodeId, 'root'),
        sourceFlowName,
    )
    const step = (id: string | null, fallback: string) => (id ? nodeLabel(id, sourceScope) : fallback)
    const node = step(nodeId, 'unknown')
    if (type === 'run_meta') {
        return `${sourcePrefix}Run details recorded`
    }
    if (type === 'PipelineStarted') {
        return `${sourcePrefix}Run started`
    }
    if (type === 'PipelineCompleted') {
        const outcome = asTrimmedString(payload.outcome)
        const reason = asTrimmedString(payload.outcome_reason_message) ?? asTrimmedString(payload.outcome_reason_code)
        const base = `${sourcePrefix}Run completed at ${step(nodeId, 'exit')}`
        if (outcome === 'failure') {
            return reason ? `${base} (failure: ${reason})` : `${base} (failure)`
        }
        return outcome ? `${base} (${outcome})` : base
    }
    if (type === 'PipelineFailed') {
        const error = asTrimmedString(payload.error)
        return error ? `${sourcePrefix}Run failed: ${error}` : `${sourcePrefix}Run failed`
    }
    if (type === 'PipelinePaused') {
        return `${sourcePrefix}Run paused at ${step(asTrimmedString(payload.current_node) ?? nodeId, 'checkpoint')}`
    }
    if (type === 'PipelineRetryStarted') {
        return `${sourcePrefix}Retry started from ${step(asTrimmedString(payload.current_node) ?? nodeId, 'checkpoint')}`
    }
    if (type === 'StageStarted') {
        return `${sourcePrefix}${node} started`
    }
    // "completed" is what the line already says; only another outcome is news.
    if (type === 'StageCompleted') {
        const outcome = asTrimmedString(payload.outcome)
        return outcome && outcome !== 'completed'
            ? `${sourcePrefix}${node} completed (${outcome})`
            : `${sourcePrefix}${node} completed`
    }
    if (type === 'StageTransition') {
        const decision = asTrimmedString(payload.decision) ?? 'unknown'
        const target = step(asTrimmedString(payload.target), 'unknown')
        const visit = asFiniteNumber(payload.visit)
        return `${sourcePrefix}${node} → ${target} on ${decision}${visit !== null && visit > 1 ? ` (visit ${visit})` : ''}`
    }
    if (type === 'StageFailed') {
        const error = asTrimmedString(payload.error)
        return error ? `${sourcePrefix}${node} failed: ${error}` : `${sourcePrefix}${node} failed`
    }
    if (type === 'StageRetrying') {
        const attempt = asFiniteNumber(payload.attempt)
        return attempt !== null
            ? `${sourcePrefix}${node} retrying (attempt ${attempt})`
            : `${sourcePrefix}${node} retrying`
    }
    if (type === 'ParallelStarted') {
        const branchCount = asFiniteNumber(payload.branch_count)
        return branchCount !== null
            ? `${sourcePrefix}Parallel fan-out started (${branchCount} branches)`
            : `${sourcePrefix}Parallel fan-out started`
    }
    if (type === 'ParallelBranchStarted') {
        return `${sourcePrefix}Parallel branch ${step(asTrimmedString(payload.branch) ?? nodeId, 'unknown')} started`
    }
    if (type === 'ParallelBranchCompleted') {
        const branchName = step(asTrimmedString(payload.branch) ?? nodeId, 'unknown')
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
    const gate = step(nodeId, 'human gate')
    if (type === 'InterviewStarted') {
        return `${sourcePrefix}${gate} asked a question`
    }
    if (type === 'InterviewInform') {
        const message = asTrimmedString(payload.message)
            ?? asTrimmedString(payload.prompt)
            ?? asTrimmedString(payload.question)
        return message ? `${sourcePrefix}Note from ${gate}: ${message}` : `${sourcePrefix}Note from ${gate}`
    }
    if (type === 'InterviewCompleted') {
        const answer = asTrimmedString(payload.answer)
        if (interviewOutcomeProvenanceFromPayload(type, payload) === 'skipped') {
            return `${sourcePrefix}Question at ${gate} skipped`
        }
        return answer ? `${sourcePrefix}Answer for ${gate}: ${answer}` : `${sourcePrefix}Question at ${gate} answered`
    }
    if (type === 'human_gate') {
        const prompt = asTrimmedString(payload.prompt)
        return prompt ? `${sourcePrefix}Waiting for an answer: ${prompt}` : `${sourcePrefix}Waiting for an answer at ${node}`
    }
    if (type === 'CheckpointSaved') {
        return `${sourcePrefix}Checkpoint saved at ${step(asTrimmedString(payload.current_node) ?? nodeId, 'current node')}`
    }
    return null
}

/** A journal entry's line, with steps named by `nodeLabel`; kinds the UI does not word keep the runtime's summary. */
const journalEntrySummary = (event: TimelineEventEntry, nodeLabel: NodeLabel): string => (
    timelineSummaryFromEvent(
        event.type,
        event.payload,
        event.nodeId,
        event.sourceScope,
        event.sourceParentNodeId,
        event.sourceFlowName,
        nodeLabel,
    ) ?? event.summary
)

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
        summary: timelineSummaryFromEvent(type, payload, nodeId, sourceScope, sourceParentNodeId, sourceFlowName)
            ?? `${timelineSourcePrefix(sourceScope, sourceParentNodeId, sourceFlowName)}${type}`,
        receivedAt: emittedAt,
        sourceScope,
        sourceParentNodeId,
        sourceFlowName,
        questionId: asTrimmedString(payload.question_id),
        payload,
    }
}

const timelineEntityKey = (event: TimelineEventEntry): string | null => {
    if (event.payload.origin === 'agent_clarification' && event.questionId) {
        return `clarification:${event.questionId}`
    }
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

const buildTimelineTypeOptions = (timelineEvents: Iterable<TimelineEventEntry>) => {
    const eventTypes = new Set<string>()
    for (const event of timelineEvents) {
        eventTypes.add(event.type)
    }
    return Array.from(eventTypes).sort((left, right) => left.localeCompare(right))
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
    const detailsValue = event.payload.details
    const details = typeof detailsValue === 'string' && detailsValue.trim().length > 0
        ? detailsValue.trim()
        : null

    return {
        eventId: event.id,
        runId: typeof event.payload.run_id === 'string' ? event.payload.run_id : undefined,
        origin: typeof event.payload.origin === 'string' ? event.payload.origin : undefined,
        sequence: event.sequence,
        receivedAt: event.receivedAt,
        nodeId: event.nodeId,
        stageIndex: event.stageIndex,
        sourceScope: event.sourceScope,
        sourceParentNodeId: event.sourceParentNodeId,
        sourceFlowName: event.sourceFlowName,
        prompt,
        details,
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
        if (event.type === 'InterviewCompleted') {
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
        if (gate.origin !== 'agent_clarification' && pendingGateKeys.has(dedupeKey)) {
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
    const pendingGates = journalPendingGates.map((gate) => ({ ...gate })).filter((gate) => gate.origin !== 'agent_clarification'
        || pendingQuestionSnapshots.some((question) => question.questionId === gate.questionId))
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
        const questionIdMatch = pendingGates.find((gate) => gate.questionId === question.questionId)
        if (questionIdMatch) {
            questionIdMatch.runId = question.runId
            continue
        }
        const dedupeKey = pendingInterviewGateDedupeKey('root', null, null, question.nodeId, question.prompt)
        if (question.origin !== 'agent_clarification' && pendingGateKeys.has(dedupeKey)) {
            continue
        }
        pendingGateKeys.add(dedupeKey)
        pendingGates.push({
            eventId: `question:${question.questionId}`,
            runId: question.runId,
            origin: question.origin,
            sequence: nextSequence,
            receivedAt: PENDING_GATE_FALLBACK_RECEIVED_AT,
            nodeId: question.nodeId,
            stageIndex: null,
            sourceScope: 'root',
            sourceParentNodeId: null,
            sourceFlowName: null,
            prompt: question.prompt,
            details: null,
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
            ...(gate.origin === 'agent_clarification' ? [gate.runId] : []),
            gate.sourceScope,
            gate.sourceParentNodeId ?? 'root',
            gate.sourceFlowName ?? '',
            gate.nodeId ?? 'human-gate',
            gate.stageIndex !== null ? String(gate.stageIndex) : 'na',
        ].join('::')
        if (!grouped.has(key)) {
            const headingNode = gate.nodeId ?? 'human gate'
            const headingStage = gate.stageIndex !== null ? ` (index ${gate.stageIndex})` : ''
            const sourceLabel = gate.origin === 'agent_clarification' && gate.runId
                ? `Run ${gate.runId}`
                : timelineSourceLabel(gate.sourceScope, gate.sourceParentNodeId, gate.sourceFlowName)
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

export {
    buildGroupedPendingInterviewGates,
    buildGroupedTimelineEntries,
    buildJournalPendingInterviewGates,
    buildPendingInterviewGates,
    buildRetryCorrelationEntityKeys,
    buildTimelineTypeOptions,
    filterAnsweredPendingInterviewGates,
    journalEntrySummary,
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
