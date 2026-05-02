export interface TokenUsageBucket {
    input_tokens: number
    cached_input_tokens: number
    output_tokens: number
    total_tokens: number
}

export interface TokenUsageBreakdown extends TokenUsageBucket {
    by_model: Record<string, TokenUsageBucket>
}

export interface ModelEstimatedCost {
    currency: string
    amount: number | null
    status: 'estimated' | 'unpriced'
}

export interface EstimatedModelCost {
    currency: string
    amount: number
    status: 'estimated' | 'partial_unpriced' | 'unpriced'
    unpriced_models: string[]
    by_model?: Record<string, ModelEstimatedCost>
}

export interface RunRecord {
    run_id: string
    flow_name: string
    status: string
    outcome?: 'success' | 'failure' | null
    outcome_reason_code?: string | null
    outcome_reason_message?: string | null
    working_directory: string
    project_path?: string
    git_branch?: string | null
    git_commit?: string | null
    spec_id?: string | null
    plan_id?: string | null
    model: string
    provider?: string | null
    llm_provider?: string | null
    reasoning_effort?: string | null
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
    token_usage_breakdown?: TokenUsageBreakdown | null
    estimated_model_cost?: EstimatedModelCost | null
    current_node?: string | null
    continued_from_run_id?: string | null
    continued_from_node?: string | null
    continued_from_flow_mode?: string | null
    continued_from_flow_name?: string | null
    parent_run_id?: string | null
    parent_node_id?: string | null
    root_run_id?: string | null
    child_invocation_index?: number | null
    execution_mode?: string
    execution_profile_id?: string
    execution_worker_id?: string
    execution_worker_label?: string
    execution_worker_base_url?: string
    execution_container_image?: string | null
    execution_mapped_project_path?: string
    execution_worker_runtime_root?: string
    execution_worker_version?: string
    execution_worker_capabilities?: unknown
    execution_profile_capabilities?: unknown
    cleanup_error?: string
}

export interface CheckpointResponse {
    pipeline_id: string
    checkpoint: Record<string, unknown>
}

export interface ContextResponse {
    pipeline_id: string
    context: Record<string, unknown>
}

export interface ArtifactListEntry {
    path: string
    size_bytes: number
    media_type: string
    viewable: boolean
}

export interface ArtifactListResponse {
    pipeline_id: string
    artifacts: ArtifactListEntry[]
}

export interface CheckpointErrorState {
    message: string
    help: string
}

export interface ContextErrorState {
    message: string
    help: string
}

export interface ArtifactErrorState {
    message: string
    help: string
}

export interface FormattedContextValue {
    renderedValue: string
    valueType: string
    renderKind: 'scalar' | 'structured'
}

export interface ContextExportEntry {
    key: string
    value: unknown
}

export interface RunContextRow extends FormattedContextValue {
    key: string
    rawValue: unknown
}

export type TimelineEventCategory =
    | 'lifecycle'
    | 'stage'
    | 'parallel'
    | 'interview'
    | 'checkpoint'
    | 'log'
    | 'runtime'
    | 'state'
    | 'metadata'
    | 'other'
export type TimelineSeverity = 'info' | 'warning' | 'error'
export type TimelineCorrelationKind = 'retry' | 'interview'
export type TimelineSourceScope = 'root' | 'child'

export interface TimelineEventEntry {
    id: string
    sequence: number
    type: string
    category: TimelineEventCategory
    severity: TimelineSeverity
    nodeId: string | null
    stageIndex: number | null
    summary: string
    receivedAt: string
    sourceScope: TimelineSourceScope
    sourceParentNodeId: string | null
    sourceFlowName: string | null
    questionId?: string | null
    payload: Record<string, unknown>
}

export interface TimelineCorrelationDescriptor {
    key: string
    kind: TimelineCorrelationKind
    label: string
}

export interface GroupedTimelineEntry {
    id: string
    correlation: TimelineCorrelationDescriptor | null
    events: TimelineEventEntry[]
}

export interface RunProgressEntry {
    id: string
    nodeId: string | null
    channel: 'assistant' | 'reasoning' | 'plan'
    status: 'streaming' | 'complete'
    content: string
    updatedAt: string
    latestSequence: number
}

export type RunProgressNodeFilter = 'current' | 'recent' | string

export interface RunProgressProjection {
    activeEntry: RunProgressEntry | null
    recentEntries: RunProgressEntry[]
    nodeOptions: string[]
}

export interface PendingQuestionOption {
    label: string
    value: string
    key: string | null
    description: string | null
}

export interface PendingInterviewGate {
    eventId: string
    sequence: number
    receivedAt: string
    nodeId: string | null
    stageIndex: number | null
    sourceScope: TimelineSourceScope
    sourceParentNodeId: string | null
    sourceFlowName: string | null
    prompt: string
    questionId: string | null
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null
    options: PendingQuestionOption[]
}

export interface PendingQuestionSnapshot {
    questionId: string
    nodeId: string | null
    prompt: string
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null
    options: PendingQuestionOption[]
}

export interface PendingInterviewGateGroup {
    key: string
    heading: string
    gates: PendingInterviewGate[]
}

export const TIMELINE_CATEGORY_LABELS: Record<TimelineEventCategory, string> = {
    lifecycle: 'Lifecycle',
    stage: 'Stage',
    parallel: 'Parallel',
    interview: 'Interview',
    checkpoint: 'Checkpoint',
    log: 'Log',
    runtime: 'Runtime',
    state: 'State',
    metadata: 'Metadata',
    other: 'Other',
}

export const TIMELINE_SEVERITY_LABELS: Record<TimelineSeverity, string> = {
    info: 'Info',
    warning: 'Warning',
    error: 'Error',
}

export const TIMELINE_SEVERITY_STYLES: Record<TimelineSeverity, string> = {
    info: 'border-border/80 bg-background text-muted-foreground',
    warning: 'border-amber-500/40 bg-amber-500/10 text-amber-800',
    error: 'border-destructive/40 bg-destructive/10 text-destructive',
}

export const RUN_JOURNAL_WINDOW_SIZE = 80

export const STATUS_STYLES: Record<string, string> = {
    running: 'bg-sky-500/15 text-sky-700',
    completed: 'bg-green-500/15 text-green-800',
    success: 'bg-green-500/15 text-green-800',
    failed: 'bg-destructive/15 text-destructive',
    fail: 'bg-destructive/15 text-destructive',
    aborted: 'bg-amber-500/15 text-amber-800',
    canceled: 'bg-amber-500/15 text-amber-800',
    paused: 'bg-amber-500/15 text-amber-800',
    pause_requested: 'bg-amber-500/15 text-amber-800',
    abort_requested: 'bg-amber-500/15 text-amber-800',
    cancel_requested: 'bg-amber-500/15 text-amber-800',
    validation_error: 'bg-destructive/15 text-destructive',
}

export const STATUS_LABELS: Record<string, string> = {
    completed: 'Completed',
    failed: 'Failed',
    pause_requested: 'Pausing',
    abort_requested: 'Canceling',
    cancel_requested: 'Canceling',
    aborted: 'Canceled',
    canceled: 'Canceled',
}

export const canCancelRun = (status: string) => status === 'running'

export const canContinueRun = (status: string) => !['running', 'cancel_requested', 'abort_requested', 'pause_requested'].includes(status)
export const canRetryRun = (status: string) => status === 'failed'

export const cancelRunActionLabel = (status: string) => {
    if (status === 'cancel_requested' || status === 'abort_requested') {
        return 'Canceling…'
    }
    if (status === 'canceled' || status === 'aborted') {
        return 'Canceled'
    }
    return 'Cancel'
}

export const cancelRunDisabledReason = (status: string) => {
    if (status === 'cancel_requested' || status === 'abort_requested') {
        return 'Cancel already requested for this run.'
    }
    if (status === 'canceled' || status === 'aborted') {
        return 'This run is already canceled.'
    }
    return 'Cancel is only available while the run is active.'
}

export const formatOutcomeLabel = (value?: string | null) => {
    if (!value) return '—'
    if (value === 'success') return 'Success'
    if (value === 'failure') return 'Failure'
    return value
}

export const formatTimestamp = (value?: string | null) => {
    if (!value) return '—'
    const date = new Date(value)
    if (Number.isNaN(date.getTime())) return '—'
    return date.toLocaleString()
}

export const formatDuration = (start?: string, end?: string | null, status?: string, now?: number) => {
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

export const pendingGateSemanticHint = (
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null,
    optionValue: string,
): string | null => {
    if (questionType !== 'YES_NO' && questionType !== 'CONFIRMATION') {
        return null
    }
    if (optionValue === 'YES' || optionValue === 'NO') {
        return `Sends ${optionValue}`
    }
    return null
}
