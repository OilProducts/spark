import { useCallback, useEffect, useMemo, useState } from 'react'
import {
    ApiHttpError,
    fetchPipelineArtifactPreviewValidated,
    fetchPipelineArtifactsValidated,
    fetchPipelineCheckpointValidated,
    fetchPipelineContextValidated,
    fetchPipelineGraphValidated,
    fetchPipelineQuestionsValidated,
    pipelineArtifactHref,
} from '@/lib/attractorClient'
import type {
    ArtifactErrorState,
    ArtifactListEntry,
    ArtifactListResponse,
    CheckpointErrorState,
    CheckpointResponse,
    ContextErrorState,
    ContextExportEntry,
    ContextResponse,
    FormattedContextValue,
    GraphvizErrorState,
    PendingQuestionOption,
    PendingQuestionSnapshot,
    RunRecord,
} from '../shared'

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

const graphvizErrorFromResponse = (status: number, detail: string | null): GraphvizErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    if (status === 404 && normalizedDetail === 'graph visualization unavailable') {
        return {
            message: 'Graph visualization unavailable for this run.',
            help: 'This run may not have produced a Graphviz SVG yet.',
        }
    }
    return {
        message: `Unable to load graph visualization (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
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

type UseRunDetailsArgs = {
    selectedRunSummary: RunRecord | null
    viewMode: string
}

export function useRunDetails({ selectedRunSummary, viewMode }: UseRunDetailsArgs) {
    const [checkpointData, setCheckpointData] = useState<CheckpointResponse | null>(null)
    const [isCheckpointLoading, setIsCheckpointLoading] = useState(false)
    const [checkpointError, setCheckpointError] = useState<CheckpointErrorState | null>(null)
    const [contextData, setContextData] = useState<ContextResponse | null>(null)
    const [isContextLoading, setIsContextLoading] = useState(false)
    const [contextError, setContextError] = useState<ContextErrorState | null>(null)
    const [contextSearchQuery, setContextSearchQuery] = useState('')
    const [contextCopyStatus, setContextCopyStatus] = useState('')
    const [artifactData, setArtifactData] = useState<ArtifactListResponse | null>(null)
    const [isArtifactLoading, setIsArtifactLoading] = useState(false)
    const [artifactError, setArtifactError] = useState<ArtifactErrorState | null>(null)
    const [selectedArtifactPath, setSelectedArtifactPath] = useState<string | null>(null)
    const [artifactViewerPayload, setArtifactViewerPayload] = useState('')
    const [artifactViewerError, setArtifactViewerError] = useState<string | null>(null)
    const [isArtifactViewerLoading, setIsArtifactViewerLoading] = useState(false)
    const [graphvizMarkup, setGraphvizMarkup] = useState('')
    const [isGraphvizLoading, setIsGraphvizLoading] = useState(false)
    const [graphvizError, setGraphvizError] = useState<GraphvizErrorState | null>(null)
    const [pendingQuestionSnapshots, setPendingQuestionSnapshots] = useState<PendingQuestionSnapshot[]>([])

    const fetchCheckpoint = useCallback(async () => {
        if (!selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        setIsCheckpointLoading(true)
        setCheckpointError(null)
        try {
            const payload = await fetchPipelineCheckpointValidated(selectedRunSummary.run_id) as CheckpointResponse
            setCheckpointData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setCheckpointData(null)
            if (err instanceof ApiHttpError) {
                setCheckpointError(checkpointErrorFromResponse(err.status, err.detail))
                return
            }
            setCheckpointError({
                message: 'Unable to load checkpoint.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsCheckpointLoading(false)
        }
    }, [selectedRunSummary])

    const fetchContext = useCallback(async () => {
        if (!selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setIsContextLoading(false)
            return
        }
        setIsContextLoading(true)
        setContextError(null)
        try {
            const payload = await fetchPipelineContextValidated(selectedRunSummary.run_id) as ContextResponse
            setContextData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setContextData(null)
            if (err instanceof ApiHttpError) {
                setContextError(contextErrorFromResponse(err.status, err.detail))
                return
            }
            setContextError({
                message: 'Unable to load context.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsContextLoading(false)
        }
    }, [selectedRunSummary])

    const fetchArtifacts = useCallback(async () => {
        if (!selectedRunSummary) {
            setArtifactData(null)
            setArtifactError(null)
            setIsArtifactLoading(false)
            return
        }
        setIsArtifactLoading(true)
        setArtifactError(null)
        try {
            const payload = await fetchPipelineArtifactsValidated(selectedRunSummary.run_id)
            setArtifactData(payload)
        } catch (err) {
            logUnexpectedRunError(err)
            setArtifactData(null)
            if (err instanceof ApiHttpError) {
                setArtifactError(artifactErrorFromResponse(err.status, err.detail))
                return
            }
            setArtifactError({
                message: 'Unable to load artifacts.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsArtifactLoading(false)
        }
    }, [selectedRunSummary])

    const fetchGraphviz = useCallback(async () => {
        if (!selectedRunSummary) {
            setGraphvizMarkup('')
            setGraphvizError(null)
            setIsGraphvizLoading(false)
            return
        }
        setIsGraphvizLoading(true)
        setGraphvizError(null)
        try {
            const svgMarkup = await fetchPipelineGraphValidated(selectedRunSummary.run_id)
            setGraphvizMarkup(svgMarkup)
        } catch (err) {
            logUnexpectedRunError(err)
            setGraphvizMarkup('')
            if (err instanceof ApiHttpError) {
                setGraphvizError(graphvizErrorFromResponse(err.status, err.detail))
                return
            }
            setGraphvizError({
                message: 'Unable to load graph visualization.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsGraphvizLoading(false)
        }
    }, [selectedRunSummary])

    const fetchPendingQuestions = useCallback(async () => {
        if (!selectedRunSummary) {
            setPendingQuestionSnapshots([])
            return
        }
        try {
            const payload = await fetchPipelineQuestionsValidated(selectedRunSummary.run_id)
            const rawQuestions = payload.questions
            if (!Array.isArray(rawQuestions)) {
                setPendingQuestionSnapshots([])
                return
            }
            const parsedQuestions = rawQuestions
                .map((question) => asPendingQuestionSnapshot(question))
                .filter((question): question is PendingQuestionSnapshot => question !== null)
            setPendingQuestionSnapshots(parsedQuestions)
        } catch (error) {
            logUnexpectedRunError(error)
            setPendingQuestionSnapshots([])
        }
    }, [selectedRunSummary])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        void fetchCheckpoint()
    }, [fetchCheckpoint, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setContextSearchQuery('')
            setContextCopyStatus('')
            setIsContextLoading(false)
            return
        }
        void fetchContext()
    }, [fetchContext, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setArtifactData(null)
            setArtifactError(null)
            setSelectedArtifactPath(null)
            setArtifactViewerPayload('')
            setArtifactViewerError(null)
            setIsArtifactLoading(false)
            setIsArtifactViewerLoading(false)
            return
        }
        void fetchArtifacts()
    }, [fetchArtifacts, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setGraphvizMarkup('')
            setGraphvizError(null)
            setIsGraphvizLoading(false)
            return
        }
        void fetchGraphviz()
    }, [fetchGraphviz, selectedRunSummary, viewMode])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setPendingQuestionSnapshots([])
            return
        }
        void fetchPendingQuestions()
    }, [fetchPendingQuestions, selectedRunSummary, viewMode])

    const checkpointSnapshot = useMemo(() => asRecord(checkpointData?.checkpoint), [checkpointData])
    const checkpointCurrentNode = useMemo(() => {
        const currentNode = checkpointSnapshot?.current_node
        return typeof currentNode === 'string' && currentNode.trim().length > 0 ? currentNode : '—'
    }, [checkpointSnapshot])
    const checkpointCompletedNodes = useMemo(() => {
        const completedNodes = checkpointSnapshot?.completed_nodes
        if (!Array.isArray(completedNodes)) {
            return '—'
        }
        const normalized = completedNodes
            .map((value) => (typeof value === 'string' ? value.trim() : ''))
            .filter((value) => value.length > 0)
        return normalized.length > 0 ? normalized.join(', ') : '—'
    }, [checkpointSnapshot])
    const checkpointRetryCounters = useMemo(() => {
        const retryCounts = asRecord(checkpointSnapshot?.retry_counts)
        if (!retryCounts) {
            return '—'
        }
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
    }, [checkpointSnapshot])
    const contextSnapshot = useMemo(() => asRecord(contextData?.context), [contextData])
    const contextRows = useMemo(() => {
        if (!contextSnapshot) {
            return []
        }
        return Object.entries(contextSnapshot)
            .map(([key, value]) => {
                const formatted = formatContextValue(value)
                return { key, rawValue: value, ...formatted }
            })
            .sort((a, b) => a.key.localeCompare(b.key))
    }, [contextSnapshot])
    const filteredContextRows = useMemo(() => {
        const normalizedQuery = contextSearchQuery.trim().toLowerCase()
        if (!normalizedQuery) {
            return contextRows
        }
        return contextRows.filter((row) => (
            row.key.toLowerCase().includes(normalizedQuery)
            || row.renderedValue.toLowerCase().includes(normalizedQuery)
        ))
    }, [contextRows, contextSearchQuery])
    const contextExportPayload = useMemo(() => {
        if (!selectedRunSummary) {
            return ''
        }
        return buildContextExportPayload(
            selectedRunSummary.run_id,
            filteredContextRows.map((row) => ({ key: row.key, value: row.rawValue })),
        )
    }, [filteredContextRows, selectedRunSummary])
    const contextExportHref = useMemo(() => {
        if (!contextExportPayload) {
            return ''
        }
        return `data:application/json;charset=utf-8,${encodeURIComponent(contextExportPayload)}`
    }, [contextExportPayload])
    const copyContextToClipboard = useCallback(async () => {
        if (!contextExportPayload || filteredContextRows.length === 0) {
            setContextCopyStatus('No context entries available to copy.')
            return
        }
        try {
            await window.navigator.clipboard.writeText(contextExportPayload)
            setContextCopyStatus('Filtered context copied.')
        } catch (error) {
            console.error(error)
            setContextCopyStatus('Copy failed. Clipboard access is unavailable.')
        }
    }, [contextExportPayload, filteredContextRows])
    const artifactEntries = useMemo(() => artifactData?.artifacts || [], [artifactData])
    const missingCoreArtifacts = useMemo(() => {
        if (artifactEntries.length === 0) {
            return []
        }
        const available = new Set(artifactEntries.map((entry) => entry.path))
        return EXPECTED_CORE_ARTIFACT_PATHS.filter((path) => !available.has(path))
    }, [artifactEntries])
    const showPartialRunArtifactNote = artifactEntries.length === 0 || missingCoreArtifacts.length > 0
    const selectedArtifactEntry = useMemo(() => {
        if (!selectedArtifactPath) {
            return null
        }
        return artifactEntries.find((entry) => entry.path === selectedArtifactPath) || null
    }, [artifactEntries, selectedArtifactPath])
    const viewArtifact = useCallback(async (entry: ArtifactListEntry) => {
        if (!selectedRunSummary) {
            return
        }
        setSelectedArtifactPath(entry.path)
        setArtifactViewerPayload('')
        setArtifactViewerError(null)
        if (!entry.viewable) {
            setArtifactViewerError('Preview unavailable for this artifact type. Use download action.')
            return
        }
        setIsArtifactViewerLoading(true)
        try {
            const payload = await fetchPipelineArtifactPreviewValidated(selectedRunSummary.run_id, entry.path)
            setArtifactViewerPayload(payload)
        } catch (error) {
            logUnexpectedRunError(error)
            if (error instanceof ApiHttpError) {
                setArtifactViewerError(artifactPreviewErrorFromResponse(error.status, error.detail))
                return
            }
            setArtifactViewerError('Unable to load artifact preview. Check your network/backend connection and retry.')
        } finally {
            setIsArtifactViewerLoading(false)
        }
    }, [selectedRunSummary])
    const artifactDownloadHref = useCallback((artifactPath: string) => {
        if (!selectedRunSummary) {
            return ''
        }
        return pipelineArtifactHref(selectedRunSummary.run_id, artifactPath, true)
    }, [selectedRunSummary])
    const graphvizViewerSrc = useMemo(() => {
        if (!graphvizMarkup) {
            return ''
        }
        return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(graphvizMarkup)}`
    }, [graphvizMarkup])

    const degradedDetailPanels = useMemo(() => {
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
        if (graphvizError) {
            panels.push('graph visualization')
        }
        return panels
    }, [artifactError, checkpointError, contextError, graphvizError])

    return {
        artifactDownloadHref,
        artifactEntries,
        artifactError,
        artifactViewerError,
        artifactViewerPayload,
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointData,
        checkpointError,
        checkpointRetryCounters,
        contextCopyStatus,
        contextError,
        contextExportHref,
        contextSearchQuery,
        degradedDetailPanels,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        fetchGraphviz,
        filteredContextRows,
        graphvizError,
        graphvizViewerSrc,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        isGraphvizLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
        showPartialRunArtifactNote,
        viewArtifact,
        copyContextToClipboard,
    }
}
