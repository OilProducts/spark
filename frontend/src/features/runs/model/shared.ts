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
    execution_container_image?: string | null
    execution_profile_capabilities?: unknown
    execution_lock?: {
        scope: string
        key: string
        conflict_policy: string
        identity: string
        state: string
        queue_position?: number | null
    } | null
    cleanup_error?: string
    launch_context?: Record<string, unknown> | null
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
    context_capture_kind?: 'assembled_messages' | 'codex_turn_input'
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

export interface PendingQuestionOption {
    label: string
    value: string
    key: string | null
    description: string | null
}

export interface PendingInterviewGate {
    runId?: string
    origin?: string

    eventId: string
    sequence: number
    receivedAt: string
    nodeId: string | null
    stageIndex: number | null
    sourceScope: TimelineSourceScope
    sourceParentNodeId: string | null
    sourceFlowName: string | null
    prompt: string
    details: string | null
    questionId: string | null
    questionType: 'MULTIPLE_CHOICE' | 'YES_NO' | 'CONFIRMATION' | 'FREEFORM' | null
    options: PendingQuestionOption[]
}

export interface PendingQuestionSnapshot {
    runId?: string
    origin?: string

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

// Human vocabulary for journal event types; raw runtime names stay available
// in the payload for anyone who needs them.
export const TIMELINE_TYPE_LABELS: Record<string, string> = {
    PipelineStarted: 'Run started',
    PipelineCompleted: 'Run completed',
    PipelineFailed: 'Run failed',
    PipelinePaused: 'Run paused',
    PipelineRetryStarted: 'Retry started',
    PipelineRetryCompleted: 'Retry completed',
    CancelRequested: 'Cancel requested',
    StageStarted: 'Stage started',
    StageCompleted: 'Stage completed',
    StageFailed: 'Stage failed',
    StageRetrying: 'Stage retrying',
    CheckpointSaved: 'Checkpoint',
    ChildRunStarted: 'Child run started',
    ChildRunCompleted: 'Child run completed',
    ParallelStarted: 'Fan-out started',
    ParallelCompleted: 'Fan-out completed',
    ParallelBranchStarted: 'Branch started',
    ParallelBranchCompleted: 'Branch completed',
    InterviewStarted: 'Gate opened',
    InterviewCompleted: 'Input received',
    human_gate: 'Waiting for input',
    HumanInterventionRequested: 'Steer requested',
    ChildInterventionRequested: 'Child steer requested',
    LLMContent: 'Assistant output',
    CodergenAdapter: 'Agent event',
    runtime: 'Status',
    lifecycle: 'Lifecycle',
    run_meta: 'Run metadata',
    state: 'Node state',
    log: 'Log',
}

export const humanizeTimelineType = (type: string): string => {
    const label = TIMELINE_TYPE_LABELS[type]
    if (label) {
        return label
    }
    // CamelCase / snake_case fallback: "SomeNewEvent" -> "Some new event".
    const spaced = type
        .replace(/_/g, ' ')
        .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
        .toLowerCase()
        .trim()
    return spaced ? spaced.charAt(0).toUpperCase() + spaced.slice(1) : type
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

export type StatusTone = 'info' | 'warning' | 'success' | 'destructive' | 'muted'

// Border color is inert unless the call site adds a `border` width class.
export const TONE_STYLES: Record<StatusTone, string> = {
    info: 'border-info/40 bg-info/10 text-info',
    warning: 'border-warning/40 bg-warning/10 text-warning',
    success: 'border-success/40 bg-success/10 text-success',
    destructive: 'border-destructive/40 bg-destructive/10 text-destructive',
    muted: 'border-border bg-muted text-muted-foreground',
}

export const TIMELINE_SEVERITY_STYLES: Record<TimelineSeverity, string> = {
    info: 'border-border/80 bg-background text-muted-foreground',
    warning: TONE_STYLES.warning,
    error: TONE_STYLES.destructive,
}

export const RUN_JOURNAL_WINDOW_SIZE = 80

// The one place that decides which run/node status reads as which tone; unknown statuses are muted.
const STATUS_TONES: Record<string, StatusTone> = {
    running: 'info',
    waiting: 'warning',
    paused: 'warning',
    pause_requested: 'warning',
    abort_requested: 'warning',
    cancel_requested: 'warning',
    completed: 'success',
    success: 'success',
    failed: 'destructive',
    fail: 'destructive',
    validation_error: 'destructive',
}

export const statusToneClassName = (status: string) => TONE_STYLES[STATUS_TONES[status] ?? 'muted']

export const STATUS_LABELS: Record<string, string> = {
    queued: 'Queued',
    waiting: 'Needs input',
    completed: 'Completed',
    failed: 'Failed',
    pause_requested: 'Pausing',
    abort_requested: 'Canceling',
    cancel_requested: 'Canceling',
    aborted: 'Canceled',
    canceled: 'Canceled',
}

export const formatRunStatusLabel = (run: Pick<RunRecord, 'status' | 'outcome' | 'outcome_reason_code'>) =>
    run.status === 'completed' && run.outcome_reason_code === 'blocked'
        ? 'Blocked'
        : STATUS_LABELS[run.status] || run.status

export const canCancelRun = (status: string) => status === 'running' || status === 'waiting'

export const canContinueRun = (status: string) => !['queued', 'running', 'waiting', 'cancel_requested', 'abort_requested', 'pause_requested'].includes(status)
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
    } else if (status === 'running' || status === 'waiting' || status === 'pause_requested' || status === 'abort_requested' || status === 'cancel_requested') {
        endMs = now ?? Date.now()
    }
    if (endMs === null) return '—'
    const delta = Math.max(0, endMs - startMs)
    if (delta < 1000) return '<1s'
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
