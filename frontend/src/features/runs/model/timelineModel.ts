import { ApiHttpError } from '@/lib/attractorClient'
import type {
    PendingInterviewGate,
    PendingQuestionOption,
    TimelineCorrelationDescriptor,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
} from './shared'

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

export {
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
}
