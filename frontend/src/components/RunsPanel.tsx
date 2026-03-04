import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Eye, OctagonX, RefreshCcw } from 'lucide-react'
import { useStore } from '@/store'
import {
    computeRunMetadataFreshness,
    formatRunMetadataLastUpdated,
    RUN_METADATA_STALE_AFTER_MS,
} from '@/lib/runMetadataFreshness'

interface RunRecord {
    run_id: string
    flow_name: string
    status: string
    result?: string | null
    working_directory: string
    project_path?: string
    git_branch?: string | null
    git_commit?: string | null
    spec_id?: string | null
    plan_id?: string | null
    model: string
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
}

interface CheckpointResponse {
    pipeline_id: string
    checkpoint: Record<string, unknown>
}

interface ContextResponse {
    pipeline_id: string
    context: Record<string, unknown>
}

interface ArtifactListEntry {
    path: string
    size_bytes: number
    media_type: string
    viewable: boolean
}

interface ArtifactListResponse {
    pipeline_id: string
    artifacts: ArtifactListEntry[]
}

interface CheckpointErrorState {
    message: string
    help: string
}

interface ContextErrorState {
    message: string
    help: string
}

interface ArtifactErrorState {
    message: string
    help: string
}

interface GraphvizErrorState {
    message: string
    help: string
}

interface FormattedContextValue {
    renderedValue: string
    valueType: string
    renderKind: 'scalar' | 'structured'
}

interface ContextExportEntry {
    key: string
    value: unknown
}

type TimelineEventCategory = 'lifecycle' | 'stage' | 'parallel' | 'interview' | 'checkpoint'
type TimelineSeverity = 'info' | 'warning' | 'error'
type TimelineCorrelationKind = 'retry' | 'interview'

interface TimelineEventEntry {
    id: string
    type: string
    category: TimelineEventCategory
    severity: TimelineSeverity
    nodeId: string | null
    stageIndex: number | null
    summary: string
    receivedAt: string
    payload: Record<string, unknown>
}

interface TimelineCorrelationDescriptor {
    key: string
    kind: TimelineCorrelationKind
    label: string
}

interface GroupedTimelineEntry {
    id: string
    correlation: TimelineCorrelationDescriptor | null
    events: TimelineEventEntry[]
}

interface PendingInterviewGate {
    eventId: string
    nodeId: string | null
    stageIndex: number | null
    prompt: string
    questionId: string | null
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null
    options: Array<{
        label: string
        value: string
        key: string | null
        description: string | null
    }>
}

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
    InterviewCompleted: 'interview',
    InterviewTimeout: 'interview',
    human_gate: 'interview',
    CheckpointSaved: 'checkpoint',
}

const TIMELINE_CATEGORY_LABELS: Record<TimelineEventCategory, string> = {
    lifecycle: 'Lifecycle',
    stage: 'Stage',
    parallel: 'Parallel',
    interview: 'Interview',
    checkpoint: 'Checkpoint',
}

const TIMELINE_SEVERITY_LABELS: Record<TimelineSeverity, string> = {
    info: 'Info',
    warning: 'Warning',
    error: 'Error',
}

const TIMELINE_SEVERITY_STYLES: Record<TimelineSeverity, string> = {
    info: 'border-border/80 bg-background text-muted-foreground',
    warning: 'border-amber-500/40 bg-amber-500/10 text-amber-700',
    error: 'border-destructive/40 bg-destructive/10 text-destructive',
}

const TIMELINE_MAX_ITEMS = 200
const RETRY_CORRELATION_EVENT_TYPES = new Set(['StageStarted', 'StageFailed', 'StageRetrying', 'StageCompleted'])

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

const asErrorDetail = (value: unknown): string | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    const detail = (value as Record<string, unknown>).detail
    if (typeof detail !== 'string') {
        return null
    }
    const trimmed = detail.trim()
    return trimmed.length > 0 ? trimmed : null
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

const encodeArtifactPath = (artifactPath: string): string => artifactPath
    .split('/')
    .map((segment) => encodeURIComponent(segment))
    .join('/')

const EXPECTED_CORE_ARTIFACT_PATHS = ['manifest.json', 'checkpoint.json']

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

const buildContextExportPayload = (
    runId: string,
    contextEntries: ContextExportEntry[]
) => JSON.stringify(
    {
        pipeline_id: runId,
        exported_at: new Date().toISOString(),
        context: Object.fromEntries(contextEntries.map((entry) => [entry.key, entry.value])),
    },
    null,
    2
)

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

    if (type === 'PipelineFailed' || type === 'StageFailed') {
        return 'error'
    }
    if (type === 'StageRetrying' || type === 'InterviewTimeout') {
        return 'warning'
    }
    return 'info'
}

const timelineSummaryFromEvent = (type: string, payload: Record<string, unknown>, nodeId: string | null): string => {
    if (type === 'PipelineStarted') {
        return `Pipeline started at ${nodeId || 'start'}`
    }
    if (type === 'PipelineCompleted') {
        return `Pipeline completed at ${nodeId || 'exit'}`
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
    if (type === 'InterviewCompleted') {
        const answer = typeof payload.answer === 'string' && payload.answer.trim().length > 0
            ? payload.answer.trim()
            : null
        return answer
            ? `Interview completed for ${nodeId || 'human gate'} (${answer})`
            : `Interview completed for ${nodeId || 'human gate'}`
    }
    if (type === 'InterviewTimeout') {
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
    retryEntityKeys: ReadonlySet<string>
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

const STATUS_STYLES: Record<string, string> = {
    running: 'bg-sky-500/15 text-sky-700',
    success: 'bg-green-500/15 text-green-700',
    failed: 'bg-destructive/15 text-destructive',
    fail: 'bg-destructive/15 text-destructive',
    aborted: 'bg-amber-500/15 text-amber-700',
    canceled: 'bg-amber-500/15 text-amber-700',
    paused: 'bg-amber-500/15 text-amber-700',
    pause_requested: 'bg-amber-500/15 text-amber-700',
    abort_requested: 'bg-amber-500/15 text-amber-700',
    cancel_requested: 'bg-amber-500/15 text-amber-700',
    validation_error: 'bg-destructive/15 text-destructive',
}

const STATUS_LABELS: Record<string, string> = {
    pause_requested: 'Pausing',
    abort_requested: 'Canceling',
    cancel_requested: 'Canceling',
    aborted: 'Canceled',
    canceled: 'Canceled',
}

const formatTimestamp = (value?: string | null) => {
    if (!value) return '—'
    const date = new Date(value)
    if (Number.isNaN(date.getTime())) return '—'
    return date.toLocaleString()
}

const formatDuration = (start?: string, end?: string | null, status?: string, now?: number) => {
    if (!start) return '—'
    const startMs = Date.parse(start)
    if (!Number.isFinite(startMs)) return '—'
    let endMs: number | null = null
    if (end) {
        const parsed = Date.parse(end)
        if (Number.isFinite(parsed)) endMs = parsed
    } else if (status === 'running' || status === 'pause_requested' || status === 'abort_requested' || status === 'cancel_requested') {
        endMs = now ?? Date.now()
    }
    if (endMs === null) return '—'
    const delta = Math.max(0, endMs - startMs)
    const seconds = Math.floor(delta / 1000)
    const minutes = Math.floor(seconds / 60)
    const hours = Math.floor(minutes / 60)
    const remSeconds = seconds % 60
    const remMinutes = minutes % 60
    if (hours > 0) return `${hours}h ${remMinutes}m`
    if (minutes > 0) return `${minutes}m`
    return `${remSeconds}s`
}

const normalizeScopePath = (value: string) => {
    const trimmed = value.trim()
    if (!trimmed) return ''
    const slashNormalized = trimmed.replace(/\\/g, '/').replace(/\/{2,}/g, '/')
    const prefix = slashNormalized.startsWith('/') ? '/' : ''
    const rawBody = prefix ? slashNormalized.slice(1) : slashNormalized
    const parts = rawBody.split('/').filter((part) => part.length > 0)
    const segments: string[] = []
    for (const part of parts) {
        if (part === '.') {
            continue
        }
        if (part === '..') {
            segments.pop()
            continue
        }
        segments.push(part)
    }
    const normalizedBody = segments.join('/')
    if (!normalizedBody && prefix) {
        return prefix
    }
    return `${prefix}${normalizedBody}`
}

const runBelongsToProjectScope = (run: RunRecord, projectPath: string) => {
    const normalizedProjectPath = normalizeScopePath(projectPath)
    if (!normalizedProjectPath) return false

    const runWorkingDirectory = normalizeScopePath(run.working_directory || '')
    if (!runWorkingDirectory) return false
    if (runWorkingDirectory === normalizedProjectPath) return true

    return runWorkingDirectory.startsWith(`${normalizedProjectPath}/`)
}

const asStringOption = (
    value: unknown
): { label: string; value: string; key: string | null; description: string | null } | null => {
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
    const options: Array<{ label: string; value: string; key: string | null; description: string | null }> = []
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
    payload: Record<string, unknown>
): 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null => {
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
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null
): Array<{ label: string; value: string; key: string | null; description: string | null }> => {
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

const pendingGateSemanticHint = (
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null,
    optionValue: string
): string | null => {
    if (questionType !== 'YES_NO' && questionType !== 'CONFIRMATION') {
        return null
    }
    if (optionValue === 'YES' || optionValue === 'NO') {
        return `Sends ${optionValue}`
    }
    return null
}

export function RunsPanel() {
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setSpecId = useStore((state) => state.setSpecId)
    const setPlanId = useStore((state) => state.setPlanId)
    const [runs, setRuns] = useState<RunRecord[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [now, setNow] = useState(() => Date.now())
    const [lastFetchedAtMs, setLastFetchedAtMs] = useState<number | null>(null)
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
    const timelineSequenceRef = useRef(0)
    const [metadataStaleAfterMs] = useState(() => {
        const override = (globalThis as typeof globalThis & { __RUNS_METADATA_STALE_AFTER_MS__?: unknown })
            .__RUNS_METADATA_STALE_AFTER_MS__
        return typeof override === 'number' && Number.isFinite(override) && override > 0
            ? override
            : RUN_METADATA_STALE_AFTER_MS
    })
    const isFetchingRef = useRef(false)

    const fetchRuns = useCallback(async () => {
        if (isFetchingRef.current) return
        isFetchingRef.current = true
        setIsLoading(true)
        setError(null)
        try {
            const res = await fetch('/runs')
            if (!res.ok) {
                throw new Error('Failed to load runs')
            }
            const data = await res.json()
            setRuns(Array.isArray(data?.runs) ? data.runs : [])
            setLastFetchedAtMs(Date.now())
        } catch (err) {
            console.error(err)
            setError('Unable to load runs')
        } finally {
            isFetchingRef.current = false
            setIsLoading(false)
        }
    }, [])

    useEffect(() => {
        if (viewMode !== 'runs') return
        void fetchRuns()
    }, [viewMode, fetchRuns])

    useEffect(() => {
        if (viewMode !== 'runs') return
        const refreshInterval = window.setInterval(() => {
            void fetchRuns()
        }, 15_000)
        return () => window.clearInterval(refreshInterval)
    }, [viewMode, fetchRuns])

    useEffect(() => {
        if (viewMode !== 'runs') return
        const interval = window.setInterval(() => setNow(Date.now()), 1000)
        return () => window.clearInterval(interval)
    }, [viewMode])

    const scopedRuns = useMemo(() => {
        if (!activeProjectPath) return []
        return runs.filter((run) => runBelongsToProjectScope(run, activeProjectPath))
    }, [runs, activeProjectPath])

    const summary = useMemo(() => {
        const total = scopedRuns.length
        const running = scopedRuns.filter(
            (run) => run.status === 'running' || run.status === 'cancel_requested' || run.status === 'abort_requested'
        ).length
        return { total, running }
    }, [scopedRuns])

    const selectedRunSummary = useMemo(() => {
        if (scopedRuns.length === 0) return null
        return scopedRuns.find((run) => run.run_id === selectedRunId) || scopedRuns[0]
    }, [scopedRuns, selectedRunId])

    const metadataFreshness = computeRunMetadataFreshness({
        isLoading,
        lastFetchedAtMs,
        nowMs: now,
        staleAfterMs: metadataStaleAfterMs,
    })
    const metadataFreshnessLabel =
        metadataFreshness === 'refreshing'
            ? 'Refreshing'
            : metadataFreshness === 'stale'
                ? 'Stale'
                : metadataFreshness === 'fresh'
                    ? 'Fresh'
                    : 'Never'
    const metadataFreshnessStyle =
        metadataFreshness === 'stale'
            ? 'border-amber-500/40 bg-amber-500/10 text-amber-700'
            : metadataFreshness === 'fresh'
                ? 'border-green-500/40 bg-green-500/10 text-green-700'
                : 'border-border bg-muted text-muted-foreground'

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
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/checkpoint`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setCheckpointData(null)
                setCheckpointError(checkpointErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.json() as CheckpointResponse
            setCheckpointData(payload)
        } catch (err) {
            console.error(err)
            setCheckpointData(null)
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
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/context`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setContextData(null)
                setContextError(contextErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.json() as ContextResponse
            setContextData(payload)
        } catch (err) {
            console.error(err)
            setContextData(null)
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
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/artifacts`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setArtifactData(null)
                setArtifactError(artifactErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.json() as ArtifactListResponse
            const artifacts = Array.isArray(payload.artifacts)
                ? payload.artifacts
                    .map((entry) => {
                        if (!entry || typeof entry !== 'object') return null
                        const candidate = entry as Partial<ArtifactListEntry>
                        const artifactPath = typeof candidate.path === 'string' ? candidate.path.trim() : ''
                        if (!artifactPath) return null
                        return {
                            path: artifactPath,
                            size_bytes: typeof candidate.size_bytes === 'number' ? candidate.size_bytes : 0,
                            media_type: typeof candidate.media_type === 'string' ? candidate.media_type : 'application/octet-stream',
                            viewable: candidate.viewable === true,
                        } satisfies ArtifactListEntry
                    })
                    .filter((entry): entry is ArtifactListEntry => entry !== null)
                : []
            setArtifactData({
                pipeline_id: payload.pipeline_id,
                artifacts,
            })
        } catch (err) {
            console.error(err)
            setArtifactData(null)
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
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/graph`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setGraphvizMarkup('')
                setGraphvizError(graphvizErrorFromResponse(res.status, detail))
                return
            }
            const svgMarkup = await res.text()
            setGraphvizMarkup(svgMarkup)
        } catch (err) {
            console.error(err)
            setGraphvizMarkup('')
            setGraphvizError({
                message: 'Unable to load graph visualization.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsGraphvizLoading(false)
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
    }, [viewMode, selectedRunSummary, fetchCheckpoint])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setIsContextLoading(false)
            setContextSearchQuery('')
            setContextCopyStatus('')
            return
        }
        setContextSearchQuery('')
        setContextCopyStatus('')
        void fetchContext()
    }, [viewMode, selectedRunSummary, fetchContext])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setArtifactData(null)
            setArtifactError(null)
            setIsArtifactLoading(false)
            setSelectedArtifactPath(null)
            setArtifactViewerPayload('')
            setArtifactViewerError(null)
            setIsArtifactViewerLoading(false)
            return
        }
        setSelectedArtifactPath(null)
        setArtifactViewerPayload('')
        setArtifactViewerError(null)
        setIsArtifactViewerLoading(false)
        void fetchArtifacts()
    }, [viewMode, selectedRunSummary, fetchArtifacts])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setGraphvizMarkup('')
            setGraphvizError(null)
            setIsGraphvizLoading(false)
            return
        }
        setGraphvizMarkup('')
        setGraphvizError(null)
        setIsGraphvizLoading(false)
        void fetchGraphviz()
    }, [viewMode, selectedRunSummary, fetchGraphviz])

    const selectedRunTimelineId = selectedRunSummary?.run_id ?? null

    useEffect(() => {
        setPendingGateActionError(null)
        setSubmittingGateIds({})
        setAnsweredGateIds({})
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

        const source = new EventSource(`/pipelines/${encodeURIComponent(selectedRunTimelineId)}/events`)
        source.onopen = () => {
            setTimelineError(null)
            setIsTimelineLive(true)
        }
        source.onmessage = (event) => {
            try {
                const payload = JSON.parse(event.data) as unknown
                const timelineEvent = toTimelineEvent(payload, timelineSequenceRef.current)
                timelineSequenceRef.current += 1
                if (!timelineEvent) {
                    return
                }
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
    }, [viewMode, selectedRunTimelineId])

    const checkpointSnapshot = useMemo(() => asRecord(checkpointData?.checkpoint), [checkpointData])
    const checkpointCurrentNode = useMemo(() => {
        const currentNode = checkpointSnapshot?.current_node
        return typeof currentNode === 'string' && currentNode.trim().length > 0 ? currentNode : '—'
    }, [checkpointSnapshot])
    const checkpointCompletedNodes = useMemo(() => {
        const completedNodes = checkpointSnapshot?.completed_nodes
        if (!Array.isArray(completedNodes)) return '—'
        const normalized = completedNodes
            .map((value) => (typeof value === 'string' ? value.trim() : ''))
            .filter((value) => value.length > 0)
        return normalized.length > 0 ? normalized.join(', ') : '—'
    }, [checkpointSnapshot])
    const checkpointRetryCounters = useMemo(() => {
        const retryCounts = asRecord(checkpointSnapshot?.retry_counts)
        if (!retryCounts) return '—'
        const pairs = Object.entries(retryCounts)
            .filter(([key]) => key.trim().length > 0)
            .map(([key, value]) => {
                if (typeof value === 'number' && Number.isFinite(value)) return `${key}: ${value}`
                if (typeof value === 'string' || typeof value === 'boolean') return `${key}: ${String(value)}`
                return `${key}: ${JSON.stringify(value)}`
            })
        return pairs.length > 0 ? pairs.join(', ') : '—'
    }, [checkpointSnapshot])
    const contextSnapshot = useMemo(() => asRecord(contextData?.context), [contextData])
    const contextRows = useMemo(() => {
        if (!contextSnapshot) return []
        return Object.entries(contextSnapshot)
            .map(([key, value]) => {
                const formatted = formatContextValue(value)
                return { key, rawValue: value, ...formatted }
            })
            .sort((a, b) => a.key.localeCompare(b.key))
    }, [contextSnapshot])
    const filteredContextRows = useMemo(() => {
        const normalizedQuery = contextSearchQuery.trim().toLowerCase()
        if (!normalizedQuery) return contextRows
        return contextRows.filter((row) => (
            row.key.toLowerCase().includes(normalizedQuery)
            || row.renderedValue.toLowerCase().includes(normalizedQuery)
        ))
    }, [contextRows, contextSearchQuery])
    const contextExportPayload = useMemo(() => {
        if (!selectedRunSummary) return ''
        return buildContextExportPayload(
            selectedRunSummary.run_id,
            filteredContextRows.map((row) => ({ key: row.key, value: row.rawValue }))
        )
    }, [selectedRunSummary, filteredContextRows])
    const contextExportHref = useMemo(() => {
        if (!contextExportPayload) return ''
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
    const artifactEntries = useMemo(() => {
        return artifactData?.artifacts || []
    }, [artifactData])
    const missingCoreArtifacts = useMemo(() => {
        if (artifactEntries.length === 0) return []
        const available = new Set(artifactEntries.map((entry) => entry.path))
        return EXPECTED_CORE_ARTIFACT_PATHS.filter((path) => !available.has(path))
    }, [artifactEntries])
    const showPartialRunArtifactNote = artifactEntries.length === 0 || missingCoreArtifacts.length > 0
    const selectedArtifactEntry = useMemo(() => {
        if (!selectedArtifactPath) return null
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
            const encodedPath = encodeArtifactPath(entry.path)
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/artifacts/${encodedPath}`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setArtifactViewerError(artifactPreviewErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.text()
            setArtifactViewerPayload(payload)
        } catch (error) {
            console.error(error)
            setArtifactViewerError('Unable to load artifact preview. Check your network/backend connection and retry.')
        } finally {
            setIsArtifactViewerLoading(false)
        }
    }, [selectedRunSummary])
    const artifactDownloadHref = useCallback((artifactPath: string) => {
        if (!selectedRunSummary) return ''
        const encodedPath = encodeArtifactPath(artifactPath)
        return `/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/artifacts/${encodedPath}?download=1`
    }, [selectedRunSummary])
    const graphvizViewerSrc = useMemo(() => {
        if (!graphvizMarkup) return ''
        return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(graphvizMarkup)}`
    }, [graphvizMarkup])
    const timelineTypeOptions = useMemo(() => {
        return Array.from(new Set(timelineEvents.map((event) => event.type))).sort((left, right) => left.localeCompare(right))
    }, [timelineEvents])
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
    const pendingInterviewGates = useMemo(() => {
        const latestInterviewEventByEntity = new Map<string, TimelineEventEntry>()
        for (const event of timelineEvents) {
            if (event.category !== 'interview') {
                continue
            }
            const entityKey = timelineEntityKey(event) || `event:${event.id}`
            if (!latestInterviewEventByEntity.has(entityKey)) {
                latestInterviewEventByEntity.set(entityKey, event)
            }
        }

        const pendingGates: PendingInterviewGate[] = []
        const pendingGateKeys = new Set<string>()
        for (const event of latestInterviewEventByEntity.values()) {
            if (event.type === 'InterviewStarted' || event.type === 'human_gate') {
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
                const prompt = typeof questionPrompt === 'string' && questionPrompt.trim().length > 0
                    ? questionPrompt.trim()
                    : typeof gatePrompt === 'string' && gatePrompt.trim().length > 0
                        ? gatePrompt.trim()
                    : event.summary
                const dedupeKey = `${event.nodeId ?? ''}::${prompt.toLowerCase()}`
                if (pendingGateKeys.has(dedupeKey)) {
                    continue
                }
                pendingGateKeys.add(dedupeKey)
                pendingGates.push({
                    eventId: event.id,
                    nodeId: event.nodeId,
                    stageIndex: event.stageIndex,
                    prompt,
                    questionId,
                    questionType,
                    options,
                })
            }
        }
        return pendingGates
    }, [timelineEvents])
    const visiblePendingInterviewGates = useMemo(
        () => pendingInterviewGates.filter((gate) => !gate.questionId || !answeredGateIds[gate.questionId]),
        [pendingInterviewGates, answeredGateIds]
    )

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
            const res = await fetch(
                `/pipelines/${encodeURIComponent(selectedRunTimelineId)}/questions/${encodeURIComponent(gate.questionId)}/answer`,
                {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        question_id: gate.questionId,
                        selected_value: selectedValue,
                    }),
                }
            )
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                const detailSuffix = detail ? `: ${detail}` : ''
                setPendingGateActionError(`Unable to submit answer (HTTP ${res.status})${detailSuffix}.`)
                return
            }
            setAnsweredGateIds((previous) => ({
                ...previous,
                [gate.questionId!]: true,
            }))
        } catch (err) {
            console.error(err)
            setPendingGateActionError('Unable to submit answer. Check connection/backend and retry.')
        } finally {
            setSubmittingGateIds((previous) => {
                const next = { ...previous }
                delete next[gate.questionId!]
                return next
            })
        }
    }, [selectedRunTimelineId])

    const openRun = (run: RunRecord) => {
        setSelectedRunId(run.run_id)
        if (run.flow_name) {
            setActiveFlow(run.flow_name)
        }
        setViewMode('execution')
    }

    const openRunArtifact = (run: RunRecord, artifactType: 'spec' | 'plan') => {
        const artifactId = artifactType === 'spec' ? run.spec_id : run.plan_id
        if (!artifactId) {
            return
        }
        if (run.project_path) {
            setActiveProjectPath(run.project_path)
        }
        if (artifactType === 'spec') {
            setSpecId(artifactId)
        } else {
            setPlanId(artifactId)
        }
        setViewMode('projects')
    }

    const requestCancel = async (runId: string, currentStatus: string) => {
        if (currentStatus !== 'running') {
            return
        }
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        setRuns((current) =>
            current.map((run) => (
                run.run_id === runId
                    ? { ...run, status: 'cancel_requested' }
                    : run
            ))
        )
        try {
            const response = await fetch(`/pipelines/${encodeURIComponent(runId)}/cancel`, { method: 'POST' })
            if (!response.ok) {
                throw new Error(`cancel failed with HTTP ${response.status}`)
            }
            fetchRuns()
        } catch (err) {
            console.error(err)
            setRuns((current) =>
                current.map((run) => (
                    run.run_id === runId
                        ? { ...run, status: currentStatus }
                        : run
                ))
            )
            window.alert('Failed to cancel run')
        }
    }

    return (
        <div data-testid="runs-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-6xl space-y-6">
                <div className="flex items-center justify-between">
                    <div className="space-y-1">
                        <h2 className="text-lg font-semibold">Run History</h2>
                        <p className="text-sm text-muted-foreground">
                            {summary.total} total runs · {summary.running} running
                        </p>
                    </div>
                    <button
                        onClick={() => void fetchRuns()}
                        data-testid="runs-refresh-button"
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-border px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                    >
                        <RefreshCcw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
                        Refresh
                    </button>
                </div>
                <div className="flex flex-wrap items-center gap-2 text-xs">
                    <span
                        data-testid="run-metadata-freshness-indicator"
                        className={`inline-flex items-center rounded-md border px-2 py-1 font-semibold uppercase tracking-wide ${metadataFreshnessStyle}`}
                    >
                        {metadataFreshnessLabel}
                    </span>
                    <span data-testid="run-metadata-last-updated" className="text-muted-foreground">
                        {formatRunMetadataLastUpdated({ lastFetchedAtMs, nowMs: now })}
                    </span>
                </div>
                {metadataFreshness === 'stale' && (
                    <div data-testid="run-metadata-stale-indicator" className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-700">
                        Run metadata may be stale. Refresh to load the latest run status.
                    </div>
                )}

                {error && (
                    <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                        {error}
                    </div>
                )}
                {!activeProjectPath && (
                    <div className="rounded-md border border-border bg-card px-3 py-2 text-sm text-muted-foreground">
                        Select an active project to view run history for that project.
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-summary-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Run Summary</h3>
                            <span className="text-xs text-muted-foreground">{selectedRunSummary.run_id}</span>
                        </div>
                        <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-2">
                            <div data-testid="run-summary-status"><span className="font-medium">Status:</span> {STATUS_LABELS[selectedRunSummary.status] || selectedRunSummary.status}</div>
                            <div data-testid="run-summary-result"><span className="font-medium">Result:</span> {selectedRunSummary.result || '—'}</div>
                            <div data-testid="run-summary-flow-name"><span className="font-medium">Flow:</span> {selectedRunSummary.flow_name || 'Untitled'}</div>
                            <div data-testid="run-summary-started-at"><span className="font-medium">Started:</span> {formatTimestamp(selectedRunSummary.started_at)}</div>
                            <div data-testid="run-summary-ended-at"><span className="font-medium">Ended:</span> {formatTimestamp(selectedRunSummary.ended_at)}</div>
                            <div data-testid="run-summary-duration"><span className="font-medium">Duration:</span> {formatDuration(selectedRunSummary.started_at, selectedRunSummary.ended_at, selectedRunSummary.status, now)}</div>
                            <div data-testid="run-summary-model"><span className="font-medium">Model:</span> {selectedRunSummary.model || 'default model'}</div>
                            <div data-testid="run-summary-working-directory" className="break-all"><span className="font-medium">Working Dir:</span> {selectedRunSummary.working_directory || '—'}</div>
                            <div data-testid="run-summary-project-path" className="break-all"><span className="font-medium">Project Path:</span> {selectedRunSummary.project_path || activeProjectPath || '—'}</div>
                            <div data-testid="run-summary-git-branch"><span className="font-medium">Git Branch:</span> {selectedRunSummary.git_branch || '—'}</div>
                            <div data-testid="run-summary-git-commit"><span className="font-medium">Git Commit:</span> {selectedRunSummary.git_commit || '—'}</div>
                            <div data-testid="run-summary-last-error" className="break-all"><span className="font-medium">Last Error:</span> {selectedRunSummary.last_error || '—'}</div>
                            <div data-testid="run-summary-token-usage"><span className="font-medium">Tokens:</span> {typeof selectedRunSummary.token_usage === 'number' ? selectedRunSummary.token_usage.toLocaleString() : '—'}</div>
                        </div>
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-checkpoint-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Checkpoint</h3>
                            <button
                                onClick={() => void fetchCheckpoint()}
                                data-testid="run-checkpoint-refresh-button"
                                className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                            >
                                {isCheckpointLoading ? 'Refreshing…' : 'Refresh'}
                            </button>
                        </div>
                        {checkpointError && (
                            <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                <div data-testid="run-checkpoint-error">{checkpointError.message}</div>
                                <div data-testid="run-checkpoint-error-help" className="text-xs text-destructive/90">
                                    {checkpointError.help}
                                </div>
                            </div>
                        )}
                        {!checkpointError && checkpointData && (
                            <div className="space-y-3">
                                <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-3">
                                    <div data-testid="run-checkpoint-current-node">
                                        <span className="font-medium">Current Node:</span> {checkpointCurrentNode}
                                    </div>
                                    <div data-testid="run-checkpoint-completed-nodes">
                                        <span className="font-medium">Completed Nodes:</span> {checkpointCompletedNodes}
                                    </div>
                                    <div data-testid="run-checkpoint-retry-counters">
                                        <span className="font-medium">Retry Counters:</span> {checkpointRetryCounters}
                                    </div>
                                </div>
                                <pre
                                    data-testid="run-checkpoint-payload"
                                    className="max-h-60 overflow-auto rounded-md border border-border/80 bg-muted/40 p-3 text-xs text-foreground"
                                >
                                    {JSON.stringify(checkpointData, null, 2)}
                                </pre>
                            </div>
                        )}
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-context-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Context</h3>
                            <div className="flex items-center gap-2">
                                <button
                                    onClick={() => {
                                        setContextCopyStatus('')
                                        void fetchContext()
                                    }}
                                    data-testid="run-context-refresh-button"
                                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                >
                                    {isContextLoading ? 'Refreshing…' : 'Refresh'}
                                </button>
                                <button
                                    type="button"
                                    onClick={() => void copyContextToClipboard()}
                                    data-testid="run-context-copy-button"
                                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                >
                                    Copy JSON
                                </button>
                                <a
                                    data-testid="run-context-export-button"
                                    href={contextExportHref || undefined}
                                    download={`run-context-${selectedRunSummary.run_id}.json`}
                                    onClick={(event) => {
                                        if (!contextExportHref) {
                                            event.preventDefault()
                                        }
                                    }}
                                    className={`inline-flex h-7 items-center rounded-md border px-2 text-[11px] font-medium ${
                                        contextExportHref
                                            ? 'border-border text-muted-foreground hover:text-foreground'
                                            : 'cursor-not-allowed border-border/60 text-muted-foreground/50'
                                    }`}
                                >
                                    Export JSON
                                </a>
                            </div>
                        </div>
                        {contextCopyStatus && (
                            <div data-testid="run-context-copy-status" className="mb-3 text-xs text-muted-foreground">
                                {contextCopyStatus}
                            </div>
                        )}
                        <div className="mb-3">
                            <input
                                value={contextSearchQuery}
                                onChange={(event) => setContextSearchQuery(event.target.value)}
                                placeholder="Search context key or value..."
                                data-testid="run-context-search-input"
                                className="h-9 w-full rounded-md border border-border bg-background px-3 text-sm text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                            />
                        </div>
                        {contextError && (
                            <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                <div data-testid="run-context-error">{contextError.message}</div>
                                <div data-testid="run-context-error-help" className="text-xs text-destructive/90">
                                    {contextError.help}
                                </div>
                            </div>
                        )}
                        {!contextError && (
                            <div className="overflow-hidden rounded-md border border-border/80">
                                <table data-testid="run-context-table" className="w-full table-fixed border-collapse text-sm">
                                    <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                                        <tr>
                                            <th className="w-2/5 px-3 py-2 font-semibold">Key</th>
                                            <th className="px-3 py-2 font-semibold">Value</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {filteredContextRows.length > 0 ? (
                                            filteredContextRows.map((row) => (
                                                <tr key={row.key} data-testid="run-context-row" className="border-t border-border/70 align-top">
                                                    <td className="px-3 py-2 font-mono text-xs text-foreground">{row.key}</td>
                                                    <td className="space-x-2 px-3 py-2 font-mono text-xs text-foreground break-all">
                                                        <span
                                                            data-testid="run-context-row-type"
                                                            className="inline-flex rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                                                        >
                                                            {row.valueType}
                                                        </span>
                                                        {row.renderKind === 'structured' ? (
                                                            <div data-testid="run-context-row-value">
                                                                <pre
                                                                    data-testid="run-context-row-value-structured"
                                                                    className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-muted/40 px-2 py-1"
                                                                >
                                                                    {row.renderedValue}
                                                                </pre>
                                                            </div>
                                                        ) : (
                                                            <span data-testid="run-context-row-value">
                                                                <span data-testid="run-context-row-value-scalar">{row.renderedValue}</span>
                                                            </span>
                                                        )}
                                                    </td>
                                                </tr>
                                            ))
                                        ) : (
                                            <tr>
                                                <td data-testid="run-context-empty" colSpan={2} className="px-3 py-4 text-sm text-muted-foreground">
                                                    No context entries match the current search.
                                                </td>
                                            </tr>
                                        )}
                                    </tbody>
                                </table>
                            </div>
                        )}
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-artifact-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Artifacts</h3>
                            <button
                                onClick={() => {
                                    void fetchArtifacts()
                                }}
                                data-testid="run-artifact-refresh-button"
                                className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                            >
                                {isArtifactLoading ? 'Refreshing…' : 'Refresh'}
                            </button>
                        </div>
                        {artifactError && (
                            <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                <div data-testid="run-artifact-error">{artifactError.message}</div>
                                <div data-testid="run-artifact-error-help" className="text-xs text-destructive/90">
                                    {artifactError.help}
                                </div>
                            </div>
                        )}
                        {!artifactError && (
                            <div className="space-y-3">
                                {showPartialRunArtifactNote && (
                                    <div
                                        data-testid="run-artifact-partial-run-note"
                                        className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700"
                                    >
                                        <div>This run may be partial or artifacts may have been pruned.</div>
                                        {missingCoreArtifacts.length > 0 && (
                                            <div className="mt-1">
                                                Missing expected files: {missingCoreArtifacts.join(', ')}.
                                            </div>
                                        )}
                                    </div>
                                )}
                                <div className="overflow-hidden rounded-md border border-border/80">
                                    <table data-testid="run-artifact-table" className="w-full table-fixed border-collapse text-sm">
                                        <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                                            <tr>
                                                <th className="w-1/2 px-3 py-2 font-semibold">Path</th>
                                                <th className="w-28 px-3 py-2 font-semibold">Type</th>
                                                <th className="w-28 px-3 py-2 font-semibold">Size</th>
                                                <th className="px-3 py-2 font-semibold">Actions</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {artifactEntries.length > 0 ? (
                                                artifactEntries.map((artifact) => (
                                                    <tr key={artifact.path} data-testid="run-artifact-row" className="border-t border-border/70 align-top">
                                                        <td className="break-all px-3 py-2 font-mono text-xs text-foreground">{artifact.path}</td>
                                                        <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{artifact.media_type}</td>
                                                        <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{artifact.size_bytes.toLocaleString()}</td>
                                                        <td className="px-3 py-2">
                                                            <div className="flex items-center gap-2">
                                                                <button
                                                                    type="button"
                                                                    data-testid="run-artifact-view-button"
                                                                    disabled={!artifact.viewable}
                                                                    onClick={() => void viewArtifact(artifact)}
                                                                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                                                                >
                                                                    View
                                                                </button>
                                                                <a
                                                                    data-testid="run-artifact-download-link"
                                                                    href={artifactDownloadHref(artifact.path) || undefined}
                                                                    download={artifact.path.split('/').pop() || 'artifact'}
                                                                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                                                >
                                                                    Download
                                                                </a>
                                                            </div>
                                                        </td>
                                                    </tr>
                                                ))
                                            ) : (
                                                <tr>
                                                    <td data-testid="run-artifact-empty" colSpan={4} className="px-3 py-4 text-sm text-muted-foreground">
                                                        No run artifacts are available yet.
                                                    </td>
                                                </tr>
                                            )}
                                        </tbody>
                                    </table>
                                </div>
                                <div data-testid="run-artifact-viewer" className="rounded-md border border-border/80 bg-muted/30 p-3">
                                    <div className="mb-2 text-xs text-muted-foreground">
                                        {selectedArtifactEntry ? `Preview: ${selectedArtifactEntry.path}` : 'Select a viewable artifact to preview.'}
                                    </div>
                                    {isArtifactViewerLoading && (
                                        <div data-testid="run-artifact-viewer-loading" className="text-xs text-muted-foreground">
                                            Loading artifact preview...
                                        </div>
                                    )}
                                    {!isArtifactViewerLoading && artifactViewerError && (
                                        <div data-testid="run-artifact-viewer-error" className="text-xs text-destructive">
                                            {artifactViewerError}
                                        </div>
                                    )}
                                    {!isArtifactViewerLoading && !artifactViewerError && artifactViewerPayload && (
                                        <pre
                                            data-testid="run-artifact-viewer-payload"
                                            className="max-h-60 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-background px-2 py-2 font-mono text-xs text-foreground"
                                        >
                                            {artifactViewerPayload}
                                        </pre>
                                    )}
                                </div>
                            </div>
                        )}
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-graphviz-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Graphviz Render</h3>
                            <button
                                onClick={() => {
                                    void fetchGraphviz()
                                }}
                                data-testid="run-graphviz-refresh-button"
                                className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                            >
                                {isGraphvizLoading ? 'Refreshing…' : 'Refresh'}
                            </button>
                        </div>
                        <div data-testid="run-graphviz-viewer" className="rounded-md border border-border/80 bg-muted/30 p-3">
                            {isGraphvizLoading && (
                                <div data-testid="run-graphviz-viewer-loading" className="text-xs text-muted-foreground">
                                    Loading graph visualization...
                                </div>
                            )}
                            {!isGraphvizLoading && graphvizError && (
                                <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                    <div data-testid="run-graphviz-viewer-error">{graphvizError.message}</div>
                                    <div data-testid="run-graphviz-viewer-error-help" className="text-xs text-destructive/90">
                                        {graphvizError.help}
                                    </div>
                                </div>
                            )}
                            {!isGraphvizLoading && !graphvizError && graphvizViewerSrc && (
                                <img
                                    data-testid="run-graphviz-viewer-image"
                                    src={graphvizViewerSrc}
                                    alt={`Graphviz render for run ${selectedRunSummary.run_id}`}
                                    className="w-full rounded-md border border-border/70 bg-background"
                                />
                            )}
                        </div>
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-event-timeline-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Event Timeline</h3>
                            <span
                                className={`inline-flex rounded border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                                    isTimelineLive
                                        ? 'border-sky-500/40 bg-sky-500/10 text-sky-700'
                                        : 'border-border bg-muted text-muted-foreground'
                                }`}
                            >
                                {isTimelineLive ? 'Live' : 'Idle'}
                            </span>
                        </div>
                        {timelineError && (
                            <div data-testid="run-event-timeline-error" className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                {timelineError}
                            </div>
                        )}
                        {!timelineError && visiblePendingInterviewGates.length > 0 && (
                            <div data-testid="run-pending-human-gates-panel" className="mb-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2">
                                <div className="text-xs font-semibold uppercase tracking-wide text-amber-700">
                                    Pending Human Gates
                                </div>
                                {pendingGateActionError && (
                                    <div
                                        data-testid="run-pending-human-gate-answer-error"
                                        className="mt-2 rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-xs text-destructive"
                                    >
                                        {pendingGateActionError}
                                    </div>
                                )}
                                <ul className="mt-2 space-y-1">
                                    {visiblePendingInterviewGates.map((gate) => (
                                        <li key={gate.eventId} data-testid="run-pending-human-gate-item" className="text-xs text-amber-900">
                                            <div>
                                                {gate.prompt}
                                                {gate.nodeId ? ` (${gate.nodeId}${gate.stageIndex !== null ? `, index ${gate.stageIndex}` : ''})` : ''}
                                            </div>
                                            {gate.questionId && gate.options.length > 0 && (
                                                <div className="mt-1 flex flex-wrap gap-1.5">
                                                    {gate.options.map((option) => (
                                                        <div key={option.value} className="space-y-1">
                                                            <button
                                                                type="button"
                                                                data-testid={`run-pending-human-gate-answer-${option.value}`}
                                                                onClick={() => {
                                                                    void submitPendingGateAnswer(gate, option.value)
                                                                }}
                                                                disabled={submittingGateIds[gate.questionId!] === true}
                                                                className="inline-flex h-6 items-center rounded border border-amber-500/50 bg-white px-2 text-[11px] font-medium text-amber-900 hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-60"
                                                            >
                                                                {option.label}
                                                            </button>
                                                            {(() => {
                                                                const semanticHint = pendingGateSemanticHint(gate.questionType, option.value)
                                                                const showMultipleChoiceMetadata = gate.questionType === 'MULTIPLE_CHOICE'
                                                                    && (option.key || option.description)
                                                                if (!showMultipleChoiceMetadata && !semanticHint) {
                                                                    return null
                                                                }
                                                                return (
                                                                    <div
                                                                        data-testid={`run-pending-human-gate-option-metadata-${option.value}`}
                                                                        className="flex items-center gap-1 text-[10px] text-amber-900/90"
                                                                    >
                                                                        {showMultipleChoiceMetadata && option.key && <span className="font-mono">[{option.key}]</span>}
                                                                        {showMultipleChoiceMetadata && option.description && <span>{option.description}</span>}
                                                                        {semanticHint && <span>{semanticHint}</span>}
                                                                    </div>
                                                                )
                                                            })()}
                                                        </div>
                                                    ))}
                                                </div>
                                            )}
                                        </li>
                                    ))}
                                </ul>
                            </div>
                        )}
                        {!timelineError && (
                            <div className="mb-3 grid gap-2 md:grid-cols-2">
                                <label className="space-y-1 text-xs text-muted-foreground">
                                    <span>Event Type</span>
                                    <select
                                        data-testid="run-event-timeline-filter-type"
                                        value={timelineTypeFilter}
                                        onChange={(event) => setTimelineTypeFilter(event.target.value)}
                                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                                    >
                                        <option value="all">All event types</option>
                                        {timelineTypeOptions.map((type) => (
                                            <option key={type} value={type}>{type}</option>
                                        ))}
                                    </select>
                                </label>
                                <label className="space-y-1 text-xs text-muted-foreground">
                                    <span>Node/Stage</span>
                                    <input
                                        data-testid="run-event-timeline-filter-node-stage"
                                        value={timelineNodeStageFilter}
                                        onChange={(event) => setTimelineNodeStageFilter(event.target.value)}
                                        placeholder="Node id or stage index..."
                                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                                    />
                                </label>
                                <label className="space-y-1 text-xs text-muted-foreground">
                                    <span>Category</span>
                                    <select
                                        data-testid="run-event-timeline-filter-category"
                                        value={timelineCategoryFilter}
                                        onChange={(event) => setTimelineCategoryFilter(event.target.value as 'all' | TimelineEventCategory)}
                                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                                    >
                                        <option value="all">All categories</option>
                                        {Object.entries(TIMELINE_CATEGORY_LABELS).map(([category, label]) => (
                                            <option key={category} value={category}>{label}</option>
                                        ))}
                                    </select>
                                </label>
                                <label className="space-y-1 text-xs text-muted-foreground">
                                    <span>Severity</span>
                                    <select
                                        data-testid="run-event-timeline-filter-severity"
                                        value={timelineSeverityFilter}
                                        onChange={(event) => setTimelineSeverityFilter(event.target.value as 'all' | TimelineSeverity)}
                                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                                    >
                                        <option value="all">All severities</option>
                                        <option value="info">Info</option>
                                        <option value="warning">Warning</option>
                                        <option value="error">Error</option>
                                    </select>
                                </label>
                            </div>
                        )}
                        {!timelineError && timelineEvents.length === 0 && (
                            <div data-testid="run-event-timeline-empty" className="rounded-md border border-border/70 bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                                No typed timeline events yet.
                            </div>
                        )}
                        {!timelineError && timelineEvents.length > 0 && filteredTimelineEvents.length === 0 && (
                            <div data-testid="run-event-timeline-empty" className="rounded-md border border-border/70 bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                                No timeline events match the current filters.
                            </div>
                        )}
                        {groupedTimelineEntries.length > 0 && (
                            <div data-testid="run-event-timeline-list" className="max-h-80 space-y-2 overflow-auto pr-1">
                                {groupedTimelineEntries.map((entry) => (
                                    <section
                                        key={entry.id}
                                        data-testid="run-event-timeline-group"
                                        className="space-y-2 rounded-md border border-border/60 bg-background/50 p-2"
                                    >
                                        {entry.correlation && (
                                            <div className="flex flex-wrap items-center justify-between gap-2">
                                                <span
                                                    data-testid="run-event-timeline-group-label"
                                                    className="inline-flex rounded border border-border/80 bg-background px-2 py-0.5 text-[11px] uppercase tracking-wide text-muted-foreground"
                                                >
                                                    {entry.correlation.label}
                                                </span>
                                                <span className="text-[11px] text-muted-foreground">
                                                    {entry.events.length} event{entry.events.length === 1 ? '' : 's'}
                                                </span>
                                            </div>
                                        )}
                                        {entry.events.map((event) => (
                                            <article
                                                key={event.id}
                                                data-testid="run-event-timeline-row"
                                                className="rounded-md border border-border/70 bg-muted/30 px-3 py-2"
                                            >
                                                <div className="flex flex-wrap items-center gap-2 text-[11px]">
                                                    <span
                                                        data-testid="run-event-timeline-row-type"
                                                        className="inline-flex rounded border border-border/80 bg-background px-1.5 py-0.5 font-semibold uppercase tracking-wide text-foreground"
                                                    >
                                                        {event.type}
                                                    </span>
                                                    <span
                                                        data-testid="run-event-timeline-row-category"
                                                        className="inline-flex rounded border border-border/80 bg-background px-1.5 py-0.5 uppercase tracking-wide text-muted-foreground"
                                                    >
                                                        {TIMELINE_CATEGORY_LABELS[event.category]}
                                                    </span>
                                                    <span
                                                        data-testid="run-event-timeline-row-severity"
                                                        className={`inline-flex rounded border px-1.5 py-0.5 uppercase tracking-wide ${TIMELINE_SEVERITY_STYLES[event.severity]}`}
                                                    >
                                                        {TIMELINE_SEVERITY_LABELS[event.severity]}
                                                    </span>
                                                    <span data-testid="run-event-timeline-row-time" className="text-muted-foreground">
                                                        {formatTimestamp(event.receivedAt)}
                                                    </span>
                                                </div>
                                                {entry.correlation && (
                                                    <p data-testid="run-event-timeline-row-correlation" className="mt-1 text-xs text-muted-foreground">
                                                        {entry.correlation.kind === 'retry' ? 'Retry correlation' : 'Interview correlation'}: {entry.correlation.label}
                                                    </p>
                                                )}
                                                <p data-testid="run-event-timeline-row-summary" className="mt-1 text-sm text-foreground">
                                                    {event.summary}
                                                </p>
                                                {event.nodeId && (
                                                    <p data-testid="run-event-timeline-row-node" className="text-xs text-muted-foreground">
                                                        Node: {event.nodeId}
                                                        {event.stageIndex !== null ? ` (index ${event.stageIndex})` : ''}
                                                    </p>
                                                )}
                                            </article>
                                        ))}
                                    </section>
                                ))}
                            </div>
                        )}
                    </div>
                )}

                <div className="rounded-md border border-border bg-card shadow-sm">
                    <div className="grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px_170px] gap-2 border-b px-4 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        <span>Status</span>
                        <span>Result</span>
                        <span>Flow</span>
                        <span>Started</span>
                        <span>Ended</span>
                        <span>Duration</span>
                        <span>Tokens</span>
                        <span>Actions</span>
                    </div>
                    {scopedRuns.length === 0 ? (
                        <div className="px-4 py-8 text-center text-sm text-muted-foreground">
                            {activeProjectPath ? 'No runs for the active project yet.' : 'No runs yet.'}
                        </div>
                    ) : (
                        <div className="divide-y">
                            {scopedRuns.map((run) => (
                                (() => {
                                    const canCancel = run.status === 'running'
                                    const cancelActionLabel = canCancel ? 'Cancel' : (
                                        run.status === 'cancel_requested' || run.status === 'abort_requested'
                                            ? 'Canceling…'
                                            : run.status === 'canceled' || run.status === 'aborted'
                                                ? 'Canceled'
                                                : 'Cancel'
                                    )
                                    const cancelDisabledReason =
                                        run.status === 'cancel_requested' || run.status === 'abort_requested'
                                            ? 'Cancel already requested for this run.'
                                            : run.status === 'canceled' || run.status === 'aborted'
                                                ? 'This run is already canceled.'
                                                : 'Cancel is only available while the run is active.'

                                    return (
                                        <div
                                            key={run.run_id}
                                            className={`grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px_170px] gap-2 px-4 py-3 text-sm ${
                                                selectedRunId === run.run_id ? 'bg-muted/40' : ''
                                            }`}
                                        >
                                            <span
                                                className={`inline-flex h-6 items-center rounded-md px-2 text-[11px] font-semibold uppercase tracking-wide ${
                                                    STATUS_STYLES[run.status] || 'bg-muted text-muted-foreground'
                                                }`}
                                            >
                                                {STATUS_LABELS[run.status] || run.status}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {run.result || '—'}
                                            </span>
                                            <div>
                                                <div className="font-medium text-foreground">
                                                    {run.flow_name || 'Untitled'}
                                                </div>
                                                <div className="text-[11px] text-muted-foreground">
                                                    {run.model || 'default model'} · {run.run_id}
                                                </div>
                                                <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
                                                    <span data-testid="run-history-row-project-path">
                                                        Project: {run.project_path || '—'}
                                                    </span>
                                                    <span data-testid="run-history-row-git-branch">
                                                        Branch: {run.git_branch || '—'}
                                                    </span>
                                                    <span data-testid="run-history-row-git-commit">
                                                        Commit: {run.git_commit || '—'}
                                                    </span>
                                                    <span data-testid="run-history-row-spec-artifact-link">
                                                        Spec artifact: {run.spec_id ? (
                                                            <button
                                                                type="button"
                                                                onClick={() => openRunArtifact(run, 'spec')}
                                                                className="font-mono text-primary underline-offset-2 hover:underline"
                                                            >
                                                                {run.spec_id}
                                                            </button>
                                                        ) : '—'}
                                                    </span>
                                                    <span data-testid="run-history-row-plan-artifact-link">
                                                        Plan artifact: {run.plan_id ? (
                                                            <button
                                                                type="button"
                                                                onClick={() => openRunArtifact(run, 'plan')}
                                                                className="font-mono text-primary underline-offset-2 hover:underline"
                                                            >
                                                                {run.plan_id}
                                                            </button>
                                                        ) : '—'}
                                                    </span>
                                                </div>
                                            </div>
                                            <span className="text-xs text-muted-foreground">
                                                {formatTimestamp(run.started_at)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {formatTimestamp(run.ended_at)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {formatDuration(run.started_at, run.ended_at, run.status, now)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {typeof run.token_usage === 'number' ? run.token_usage.toLocaleString() : '—'}
                                            </span>
                                            <div className="flex items-center gap-2">
                                                <button
                                                    onClick={() => openRun(run)}
                                                    className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                                >
                                                    <Eye className="h-3.5 w-3.5" />
                                                    Open
                                                </button>
                                                <button
                                                    onClick={() => requestCancel(run.run_id, run.status)}
                                                    disabled={!canCancel}
                                                    title={canCancel ? undefined : cancelDisabledReason}
                                                    className="inline-flex h-7 items-center gap-1.5 rounded-md bg-destructive px-2 text-[11px] font-semibold text-destructive-foreground hover:bg-destructive/90 disabled:pointer-events-none disabled:opacity-50"
                                                >
                                                    <OctagonX className="h-3.5 w-3.5" />
                                                    {cancelActionLabel}
                                                </button>
                                            </div>
                                        </div>
                                    )
                                })()
                            ))}
                        </div>
                    )}
                </div>
            </div>
        </div>
    )
}
