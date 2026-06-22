import { ApiHttpError } from '@/lib/attractorClient'
import type {
    ArtifactErrorState,
    ArtifactListResponse,
    CheckpointErrorState,
    CheckpointResponse,
    ContextErrorState,
    ContextExportEntry,
    ContextResponse,
    FormattedContextValue,
    PendingQuestionOption,
    PendingQuestionSnapshot,
    RunContextRow,
} from './shared'

const EXPECTED_CORE_ARTIFACT_PATHS = ['manifest.json', 'checkpoint.json']

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
): PendingQuestionSnapshot['questionType'] => {
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
    questionType: PendingQuestionSnapshot['questionType'],
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

const asPendingQuestionSnapshot = (value: unknown): PendingQuestionSnapshot | null => {
    const payload = asRecord(value)
    if (!payload) {
        return null
    }
    const questionIdValue = payload.question_id
    const questionId = typeof questionIdValue === 'string' ? questionIdValue.trim() : ''
    if (!questionId) {
        return null
    }
    const promptValue = payload.prompt
    const questionPromptValue = payload.question
    const messageValue = payload.message
    const prompt = typeof promptValue === 'string' && promptValue.trim().length > 0
        ? promptValue.trim()
        : typeof questionPromptValue === 'string' && questionPromptValue.trim().length > 0
            ? questionPromptValue.trim()
            : typeof messageValue === 'string' && messageValue.trim().length > 0
                ? messageValue.trim()
                : `Question ${questionId}`
    const nodeIdValue = payload.node_id
    const nodeId = typeof nodeIdValue === 'string' && nodeIdValue.trim().length > 0 ? nodeIdValue.trim() : null
    const questionType = pendingGateQuestionTypeFromPayload(payload)
    const payloadOptions = pendingGateOptionsFromPayload(payload)
    const options = payloadOptions.length > 0
        ? payloadOptions
        : pendingGateSemanticFallbackOptions(questionType)
    return {
        questionId,
        nodeId,
        prompt,
        questionType,
        options,
    }
}

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

const checkpointErrorFromResponse = (status: number, detail: string | null): CheckpointErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'checkpoint unavailable') {
        return {
            message: 'Checkpoint unavailable for this run.',
            help: 'Run may still be in progress or did not persist checkpoint data yet.',
        }
    }
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    return {
        message: `Unable to load checkpoint (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
}

const contextErrorFromResponse = (status: number, detail: string | null): ContextErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'context unavailable') {
        return {
            message: 'Context unavailable for this run.',
            help: 'Run may still be in progress or did not persist context data yet.',
        }
    }
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    return {
        message: `Unable to load context (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
}

const artifactErrorFromResponse = (status: number, detail: string | null): ArtifactErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    return {
        message: `Unable to load artifacts (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
}

const artifactPreviewErrorFromResponse = (status: number, detail: string | null): string => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'artifact not found') {
        return 'Artifact preview unavailable because the file was not found for this run. This run may be partial or artifacts may have been pruned.'
    }
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return 'Artifact preview unavailable because this run is no longer available. Refresh run history and pick a different run.'
    }
    return detail
        ? `Unable to load artifact preview (HTTP ${status}): ${detail}.`
        : `Unable to load artifact preview (HTTP ${status}).`
}

const formatContextValue = (value: unknown): FormattedContextValue => {
    if (value === null) {
        return {
            renderedValue: 'null',
            valueType: 'null',
            renderKind: 'scalar',
        }
    }
    if (typeof value === 'string') {
        return {
            renderedValue: JSON.stringify(value),
            valueType: 'string',
            renderKind: 'scalar',
        }
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
        return {
            renderedValue: String(value),
            valueType: typeof value,
            renderKind: 'scalar',
        }
    }
    const valueType = Array.isArray(value) ? 'array' : 'object'
    let renderedValue = ''
    try {
        renderedValue = JSON.stringify(value, null, 2) ?? String(value)
    } catch {
        renderedValue = String(value)
    }
    return {
        renderedValue,
        valueType,
        renderKind: 'structured',
    }
}

const buildContextExportPayload = (runId: string, contextEntries: ContextExportEntry[]) => JSON.stringify(
    {
        pipeline_id: runId,
        exported_at: new Date().toISOString(),
        context: Object.fromEntries(contextEntries.map((entry) => [entry.key, entry.value])),
    },
    null,
    2,
)

const buildCheckpointSummary = (checkpointData: CheckpointResponse | null) => {
    const checkpointSnapshot = asRecord(checkpointData?.checkpoint)
    const currentNode = checkpointSnapshot?.current_node
    const checkpointCurrentNode = typeof currentNode === 'string' && currentNode.trim().length > 0 ? currentNode : '—'

    const completedNodes = checkpointSnapshot?.completed_nodes
    const checkpointCompletedNodes = Array.isArray(completedNodes)
        ? (() => {
            const normalized = completedNodes
                .map((value) => (typeof value === 'string' ? value.trim() : ''))
                .filter((value) => value.length > 0)
            return normalized.length > 0 ? normalized.join(', ') : '—'
        })()
        : '—'

    const retryCounts = asRecord(checkpointSnapshot?.retry_counts)
    const checkpointRetryCounters = retryCounts
        ? (() => {
            const pairs = Object.entries(retryCounts)
                .filter(([key]) => key.trim().length > 0)
                .map(([key, value]) => {
                    if (typeof value === 'number' && Number.isFinite(value)) {
                        return `${key}: ${value}`
                    }
                    if (typeof value === 'string' || typeof value === 'boolean') {
                        return `${key}: ${String(value)}`
                    }
                    return `${key}: ${JSON.stringify(value)}`
                })
            return pairs.length > 0 ? pairs.join(', ') : '—'
        })()
        : '—'

    return {
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointRetryCounters,
    }
}

const buildContextRows = (contextData: ContextResponse | null): RunContextRow[] => {
    const contextSnapshot = asRecord(contextData?.context)
    if (!contextSnapshot) {
        return []
    }
    return Object.entries(contextSnapshot)
        .map(([key, value]) => {
            const formatted = formatContextValue(value)
            return { key, rawValue: value, ...formatted }
        })
        .sort((a, b) => a.key.localeCompare(b.key))
}

const filterContextRows = (contextRows: RunContextRow[], contextSearchQuery: string) => {
    const normalizedQuery = contextSearchQuery.trim().toLowerCase()
    if (!normalizedQuery) {
        return contextRows
    }
    return contextRows.filter((row) => (
        row.key.toLowerCase().includes(normalizedQuery)
        || row.renderedValue.toLowerCase().includes(normalizedQuery)
    ))
}

const buildArtifactDerivedState = (
    artifactData: ArtifactListResponse | null,
    selectedArtifactPath: string | null,
) => {
    const artifactEntries = artifactData?.artifacts || []
    const missingCoreArtifacts = artifactEntries.length === 0
        ? []
        : (() => {
            const available = new Set(artifactEntries.map((entry) => entry.path))
            return EXPECTED_CORE_ARTIFACT_PATHS.filter((path) => !available.has(path))
        })()
    const selectedArtifactEntry = selectedArtifactPath
        ? artifactEntries.find((entry) => entry.path === selectedArtifactPath) || null
        : null

    return {
        artifactEntries,
        missingCoreArtifacts,
        selectedArtifactEntry,
        showPartialRunArtifactNote: artifactEntries.length === 0 || missingCoreArtifacts.length > 0,
    }
}

const buildDegradedDetailPanels = ({
    checkpointError,
    contextError,
    artifactError,
}: {
    checkpointError: CheckpointErrorState | null
    contextError: ContextErrorState | null
    artifactError: ArtifactErrorState | null
}) => {
    const panels: string[] = []
    if (checkpointError) {
        panels.push('checkpoint')
    }
    if (contextError) {
        panels.push('context')
    }
    if (artifactError) {
        panels.push('artifacts')
    }
    return panels
}

export {
    EXPECTED_CORE_ARTIFACT_PATHS,
    artifactErrorFromResponse,
    artifactPreviewErrorFromResponse,
    asPendingQuestionSnapshot,
    buildArtifactDerivedState,
    buildContextExportPayload,
    buildContextRows,
    buildDegradedDetailPanels,
    buildCheckpointSummary,
    checkpointErrorFromResponse,
    contextErrorFromResponse,
    filterContextRows,
    formatContextValue,
    logUnexpectedRunError,
}
