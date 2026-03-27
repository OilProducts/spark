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
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
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

export type TimelineEventCategory = 'lifecycle' | 'stage' | 'parallel' | 'interview' | 'checkpoint'
export type TimelineSeverity = 'info' | 'warning' | 'error'
export type TimelineCorrelationKind = 'retry' | 'interview'

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
    eventId: string
    sequence: number
    receivedAt: string
    nodeId: string | null
    stageIndex: number | null
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

export const TIMELINE_MAX_ITEMS = 200

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
