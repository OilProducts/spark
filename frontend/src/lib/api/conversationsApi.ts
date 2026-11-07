import { modelSettingsForApi, parseModelSettingsView, type ModelSettings, type ModelSettingsView } from './settingsApi'
import {
    ApiHttpError,
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
    extractHttpError,
    parseJsonPayload,
} from './shared'
import { fetchWorkspaceJsonValidated, workspaceUrl } from './apiClient'

/** An uploaded conversation file; sent user turns carry these references. */
export interface ConversationAttachmentResponse {
    id: string
    filename: string
    media_type: string
    size_bytes: number
}

/** Server-enforced limits, mirrored so the composer can refuse early. */
export const CONVERSATION_ATTACHMENT_LIMITS = {
    maxFiles: 10,
    maxFileBytes: 20 * 1024 * 1024,
    maxMessageBytes: 50 * 1024 * 1024,
} as const

export interface ConversationTurnResponse {
    id: string
    role: 'user' | 'assistant' | 'system'
    content: string
    timestamp: string
    /** `queued`: a message sent during a turn, waiting to start after it. */
    status: 'pending' | 'streaming' | 'complete' | 'failed' | 'queued'
    kind: 'message' | 'mode_change' | 'mission_notice' | 'flow_run_request' | 'flow_launch'
    artifact_id?: string | null
    parent_turn_id?: string | null
    error?: string | null
    token_usage?: Record<string, unknown> | null
    attachments?: ConversationAttachmentResponse[]
}

export type ConversationChatMode = 'chat' | 'plan'

export interface ConversationSegmentResponse {
    id: string
    turn_id: string
    order: number
    /** `user_message`: a message delivered into the running turn. */
    kind: 'assistant_message' | 'plan' | 'reasoning' | 'tool_call' | 'context_compaction' | 'request_user_input' | 'flow_run_request' | 'flow_launch' | 'user_message' | 'agent_event'
    /** An `agent_event`'s category; only `warning` is shown. */
    category?: string | null
    role: 'assistant' | 'system' | 'user'
    /** `delivery_pending`: a message accepted into a running turn whose delivery is unresolved. */
    status: 'pending' | 'streaming' | 'complete' | 'failed' | 'running' | 'delivery_pending'
    timestamp: string
    updated_at: string
    content: string
    completed_at?: string | null
    error?: string | null
    artifact_id?: string | null
    phase?: string | null
    tool_call?: {
        kind: 'command_execution' | 'file_change' | 'dynamic_tool' | 'subagent'
        status: 'running' | 'completed' | 'failed' | 'yielded'
        completion_reason?: string | null
        id: string
        title: string
        command?: string | null
        output?: string | null
        output_size?: number | null
        output_truncated?: boolean
        file_paths: string[]
        /** The call that spawned this one, for a subagent's own calls. */
        parent_call_id?: string | null
        subagent_type?: string | null
        prompt?: string | null
        progress?: string | null
        summary?: string | null
        /** How many tool calls a subagent reported making. */
        tool_uses?: number | null
    } | null
    request_user_input?: {
        request_id: string
        status: 'pending' | 'answered' | 'expired'
        questions: Array<{
            id: string
            header: string
            question: string
            question_type: 'MULTIPLE_CHOICE' | 'FREEFORM'
            options: Array<{
                label: string
                description?: string | null
            }>
            allow_other: boolean
            is_secret: boolean
        }>
        answers: Record<string, string>
        submitted_at?: string | null
    } | null
    source?: {
        app_turn_id?: string | null
        item_id?: string | null
        summary_index?: number | null
        call_id?: string | null
    } | null
}

export interface WorkflowEventResponse {
    message: string
    timestamp: string
    kind?: string
    error_code?: string
    details?: Record<string, unknown>
}

export interface FlowRunRequestResponse {
    id: string
    created_at: string
    updated_at: string
    flow_name: string
    summary: string
    project_path: string
    conversation_id: string
    source_turn_id: string
    status: 'pending' | 'approved' | 'rejected' | 'launch_failed' | 'launched'
    source_segment_id?: string | null
    goal?: string | null
    launch_context?: Record<string, unknown> | null
    model?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    run_id?: string | null
    launch_error?: string | null
    review_message?: string | null
}

export interface FlowLaunchResponse {
    id: string
    created_at: string
    updated_at: string
    flow_name: string
    summary: string
    project_path: string
    conversation_id: string
    source_turn_id: string
    status: 'pending' | 'launch_failed' | 'launched'
    source_segment_id?: string | null
    goal?: string | null
    launch_context?: Record<string, unknown> | null
    model?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    run_id?: string | null
    launch_error?: string | null
}

export interface ProposedPlanArtifactResponse {
    id: string
    created_at: string
    updated_at: string
    title: string
    content: string
    project_path: string
    conversation_id: string
    source_turn_id: string
    status: 'pending_review' | 'approved' | 'rejected' | 'launch_failed'
    source_segment_id?: string | null
    review_note?: string | null
    written_change_request_path?: string | null
    flow_launch_id?: string | null
    run_id?: string | null
    launch_error?: string | null
}

export interface ConversationSummaryResponse {
    conversation_id: string
    conversation_handle?: string | null
    project_path: string
    title: string
    created_at: string
    updated_at: string
    /** The conversation row's revision, a decimal string. */
    revision: string
    last_message_preview?: string | null
    /** The runs this chat launched, oldest first. */
    launched_run_ids?: string[]
    /** Set on a standing role's conversation, pinned first in the Threads list. */
    standing_role?: StandingRoleSummary | null
    /** An assistant turn is pending or streaming. */
    running?: boolean
}

export interface StandingRoleSummary {
    name: string
    title: string
    state: 'idle' | 'working' | 'stopped'
}

function parseStandingRole(value: unknown): StandingRoleSummary | null {
    const record = asUnknownRecord(value)
    if (!record || typeof record.name !== 'string' || typeof record.title !== 'string') return null
    const state = record.state === 'working' || record.state === 'stopped' ? record.state : 'idle'
    return { name: record.name, title: record.title, state }
}

export interface ConversationDeleteResponse {
    status: 'deleted'
    conversation_id: string
    conversation_handle?: string | null
    project_path: string
}

export interface ConversationToolOutputResponse {
    output: string
    output_size: number
}

export interface ConversationStreamDeltaEventResponse {
    type: 'stream_delta'
    conversation_id: string
    turn_id: string
    stream_sequence: number
    base_revision: number
    delta_kind: 'segment_text' | 'thinking'
    /** `segment_text`: the open segment the frame's text belongs to. */
    segment_id?: string
    /**
     * `segment_text`: the segment's whole display text so far, replacing what it showed: a tool
     * call's output, else its content. Never an appended fragment.
     */
    text?: string
    active?: boolean
    estimated_tokens?: number | null
    elapsed_ms?: number | null
}

/** Metadata and summary of a conversation; never its transcript. */
export interface ConversationRowPayload {
    handle: string | null
    project_path: string
    title: string
    created_at: string
    updated_at: string
    preview: string | null
    running: boolean
    needs_attention: boolean
    launched_run_ids: string[]
    chat_mode: ConversationChatMode
    /** The effective provider, model and effort; conversation scope only. */
    provider: string | null
    model: string | null
    reasoning_effort: string | null
    model_settings_view?: ModelSettingsView
    /**
     * The revision of the conversation's last committed settings change: a settings write's
     * `expected_revision`. It can trail the row's revision, which other changes also bump.
     */
    settings_revision: string | null
    /** Summary scope only; `undefined` when the row doesn't carry it. */
    standing_role?: StandingRoleSummary | null
    /** A standing role without a conversation yet. */
    placeholder: boolean
}

export type ConversationArtifactCollection = 'flow_run_requests' | 'flow_launches' | 'run_recoveries' | 'proposed_plans' | 'event_log'

type ConversationRowBase = {
    conversation_id: string
    id: string
    turn_id?: string
    position?: string
    order?: string
    revision: string
    deleted: boolean
}

/** One stored entity at its last-change revision; a tombstone has no payload. */
export type ConversationRow =
    | (ConversationRowBase & { kind: 'conversation'; payload: ConversationRowPayload | null })
    | (ConversationRowBase & { kind: 'turn'; payload: ConversationTurnResponse | null })
    | (ConversationRowBase & { kind: 'segment'; payload: ConversationSegmentResponse | null })
    | (ConversationRowBase & { kind: 'attachment'; payload: ConversationAttachmentResponse | null })
    | (ConversationRowBase & {
        kind: 'artifact'
        collection: ConversationArtifactCollection
        payload: FlowRunRequestResponse | FlowLaunchResponse | ProposedPlanArtifactResponse | WorkflowEventResponse | Record<string, unknown> | null
    })

export interface ConversationBatch {
    generation: string
    from_revision: string
    watermark: string
    rows: ConversationRow[]
}

export interface ConversationPage {
    generation: string
    watermark: string
    conversation_id: string
    rows: ConversationRow[]
    turn_ids: string[]
    pinned_turn_ids: string[]
    pinned_artifact_ids: Array<{ collection: string; id: string }>
    older_cursor: string | null
}

export interface ConversationSummarySnapshot {
    generation: string
    watermark: string
    /** `null`: every project. */
    project_path: string | null
    rows: ConversationRow[]
}

export interface ConversationMutationAck {
    generation: string
    revision: string
    changes: ConversationBatch
}

export interface ConversationAcceptance extends ConversationMutationAck {
    user_turn_id: string
    client_turn_id: string
    assistant_turn_id: string | null
    status: 'accepted' | 'queued' | 'delivery_pending'
}

export interface ConversationReset {
    code: 'window_reset' | 'resync_required'
    reason: 'order_changed' | 'generation_changed' | 'cursor_ahead' | 'catchup_limit'
    generation: string
    watermark: string
}

/** A 409 whose body says the cursor is invalid: reload from the newest window. */
export class ConversationResetError extends ApiHttpError {
    reset: ConversationReset

    constructor(endpoint: string, reset: ConversationReset) {
        super(endpoint, 409, `${reset.code}: ${reset.reason}`)
        this.name = 'ConversationResetError'
        this.reset = reset
    }
}

const DECIMAL = /^(0|[1-9][0-9]*)$/

/** Orders decimal revision strings, which can exceed a JSON number's precision. */
export function compareRevisions(left: string, right: string): number {
    const a = BigInt(left)
    const b = BigInt(right)
    return a < b ? -1 : a > b ? 1 : 0
}

function expectDecimal(value: unknown, endpoint: string, field: string): string {
    if (typeof value !== 'string' || !DECIMAL.test(value)) {
        throw new ApiSchemaError(endpoint, `Expected "${field}" to be a decimal string.`)
    }
    return value
}

function parseConversationTurnResponse(value: unknown): ConversationTurnResponse | null {
    const record = asUnknownRecord(value)
    if (!record) {
        return null
    }
    const role = typeof record.role === 'string' ? record.role : ''
    if (role !== 'user' && role !== 'assistant' && role !== 'system') {
        return null
    }
    const kind = record.kind === 'flow_run_request'
        || record.kind === 'flow_launch'
        || record.kind === 'mode_change'
        || record.kind === 'mission_notice'
        || record.kind === 'message'
        ? record.kind
        : 'message'
    if (typeof record.id !== 'string' || typeof record.content !== 'string' || typeof record.timestamp !== 'string') {
        return null
    }
    const status = record.status === 'pending'
        || record.status === 'streaming'
        || record.status === 'failed'
        || record.status === 'queued'
        ? record.status
        : 'complete'
    const attachments = Array.isArray(record.attachments)
        ? record.attachments
            .map((entry) => parseConversationAttachment(entry))
            .filter((entry): entry is ConversationAttachmentResponse => entry !== null)
        : []
    return {
        id: record.id,
        role,
        content: record.content,
        timestamp: record.timestamp,
        status,
        kind,
        artifact_id: asOptionalNullableString(record.artifact_id),
        parent_turn_id: asOptionalNullableString(record.parent_turn_id),
        error: asOptionalNullableString(record.error),
        token_usage: asUnknownRecord(record.token_usage),
        ...(attachments.length > 0 ? { attachments } : {}),
    }
}

function parseConversationAttachment(value: unknown): ConversationAttachmentResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.filename !== 'string'
        || typeof record.media_type !== 'string'
        || typeof record.size_bytes !== 'number'
    ) {
        return null
    }
    return {
        id: record.id,
        filename: record.filename,
        media_type: record.media_type,
        size_bytes: record.size_bytes,
    }
}

export function parseConversationSegmentResponse(value: unknown): ConversationSegmentResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.turn_id !== 'string'
        || typeof record.order !== 'number'
        || typeof record.kind !== 'string'
        || typeof record.role !== 'string'
        || typeof record.status !== 'string'
        || typeof record.timestamp !== 'string'
        || typeof record.updated_at !== 'string'
        || typeof record.content !== 'string'
    ) {
        return null
    }
    const kind = record.kind === 'assistant_message'
        || record.kind === 'plan'
        || record.kind === 'reasoning'
        || record.kind === 'tool_call'
        || record.kind === 'context_compaction'
        || record.kind === 'request_user_input'
        || record.kind === 'flow_run_request'
        || record.kind === 'flow_launch'
        || record.kind === 'user_message'
        || record.kind === 'agent_event'
        ? record.kind
        : null
    if (!kind) {
        return null
    }
    const role = record.role === 'system' || record.role === 'user' ? record.role : 'assistant'
    const status = record.status === 'pending'
        || record.status === 'streaming'
        || record.status === 'failed'
        || record.status === 'running'
        || record.status === 'delivery_pending'
        ? record.status
        : 'complete'
    const toolCall = asUnknownRecord(record.tool_call)
    const requestUserInput = asUnknownRecord(record.request_user_input)
    const source = asUnknownRecord(record.source)
    const requestUserInputAnswers = requestUserInput ? asUnknownRecord(requestUserInput.answers) : null
    return {
        id: record.id,
        turn_id: record.turn_id,
        order: record.order,
        kind,
        role,
        status,
        timestamp: record.timestamp,
        updated_at: record.updated_at,
        content: record.content,
        completed_at: asOptionalNullableString(record.completed_at),
        error: asOptionalNullableString(record.error),
        artifact_id: asOptionalNullableString(record.artifact_id),
        phase: asOptionalNullableString(record.phase),
        category: asOptionalNullableString(record.category),
        tool_call: toolCall && typeof toolCall.title === 'string' && typeof toolCall.id === 'string'
            ? {
                id: toolCall.id,
                kind: toolCall.kind === 'file_change' || toolCall.kind === 'dynamic_tool' || toolCall.kind === 'subagent'
                    ? toolCall.kind
                    : 'command_execution',
                // A payload without its own status takes the segment's.
                status: toolCall.status === 'running' || toolCall.status === 'failed' || toolCall.status === 'yielded'
                    ? toolCall.status
                    : typeof toolCall.status !== 'string' && status !== 'complete'
                        ? (status === 'failed' ? 'failed' : 'running')
                        : 'completed',
                completion_reason: asOptionalNullableString(toolCall.completion_reason),
                title: toolCall.title,
                command: asOptionalNullableString(toolCall.command),
                output: asOptionalNullableString(toolCall.output),
                output_size: typeof toolCall.output_size === 'number' ? toolCall.output_size : null,
                output_truncated: toolCall.output_truncated === true,
                file_paths: Array.isArray(toolCall.file_paths) ? toolCall.file_paths.map((entry) => String(entry)) : [],
                parent_call_id: asOptionalNullableString(toolCall.parent_call_id),
                subagent_type: asOptionalNullableString(toolCall.subagent_type),
                prompt: asOptionalNullableString(toolCall.prompt),
                progress: asOptionalNullableString(toolCall.progress),
                summary: asOptionalNullableString(toolCall.summary),
                tool_uses: typeof toolCall.tool_uses === 'number' ? toolCall.tool_uses : null,
            }
            : null,
        request_user_input: requestUserInput && typeof requestUserInput.request_id === 'string'
            ? {
                request_id: requestUserInput.request_id,
                status: requestUserInput.status === 'answered'
                    ? 'answered'
                    : requestUserInput.status === 'expired'
                        ? 'expired'
                        : 'pending',
                questions: Array.isArray(requestUserInput.questions)
                    ? requestUserInput.questions
                        .map((entry) => asUnknownRecord(entry))
                        .filter((entry): entry is Record<string, unknown> => (
                            entry !== null
                            && typeof entry.id === 'string'
                            && typeof entry.question === 'string'
                        ))
                        .map((entry) => ({
                            id: String(entry.id),
                            header: typeof entry.header === 'string' ? entry.header : '',
                            question: String(entry.question),
                            question_type: entry.question_type === 'MULTIPLE_CHOICE' ? 'MULTIPLE_CHOICE' : 'FREEFORM',
                            options: Array.isArray(entry.options)
                                ? entry.options
                                    .map((option) => asUnknownRecord(option))
                                    .filter((option): option is Record<string, unknown> => option !== null && typeof option.label === 'string')
                                    .map((option) => ({
                                        label: String(option.label),
                                        description: asOptionalNullableString(option.description),
                                    }))
                                : [],
                            allow_other: entry.allow_other === true,
                            is_secret: entry.is_secret === true,
                        }))
                    : [],
                answers: requestUserInputAnswers
                    ? Object.fromEntries(
                        Object.entries(requestUserInputAnswers)
                            .filter(([, answer]) => answer !== null && answer !== undefined)
                            .map(([questionId, answer]) => [questionId, String(answer)]),
                    )
                    : {},
                submitted_at: asOptionalNullableString(requestUserInput.submitted_at),
            }
            : null,
        source: source
            ? {
                app_turn_id: asOptionalNullableString(source.app_turn_id),
                item_id: asOptionalNullableString(source.item_id),
                summary_index: typeof source.summary_index === 'number' ? source.summary_index : null,
                call_id: asOptionalNullableString(source.call_id),
            }
            : null,
    }
}

function parseWorkflowEventResponse(value: unknown): WorkflowEventResponse | null {
    const record = asUnknownRecord(value)
    if (!record || typeof record.message !== 'string' || typeof record.timestamp !== 'string') {
        return null
    }
    const details = asUnknownRecord(record.details)
    return {
        message: record.message,
        timestamp: record.timestamp,
        kind: asOptionalString(record.kind),
        error_code: asOptionalString(record.error_code),
        details: details ?? undefined,
    }
}

function parseFlowRunRequestResponse(value: unknown): FlowRunRequestResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.created_at !== 'string'
        || typeof record.updated_at !== 'string'
        || typeof record.flow_name !== 'string'
        || typeof record.summary !== 'string'
        || typeof record.project_path !== 'string'
        || typeof record.conversation_id !== 'string'
        || typeof record.source_turn_id !== 'string'
    ) {
        return null
    }
    const status = record.status === 'approved'
        || record.status === 'rejected'
        || record.status === 'launch_failed'
        || record.status === 'launched'
        ? record.status
        : 'pending'
    return {
        id: record.id,
        created_at: record.created_at,
        updated_at: record.updated_at,
        flow_name: record.flow_name,
        summary: record.summary,
        project_path: record.project_path,
        conversation_id: record.conversation_id,
        source_turn_id: record.source_turn_id,
        status,
        source_segment_id: asOptionalNullableString(record.source_segment_id),
        goal: asOptionalNullableString(record.goal),
        launch_context: asUnknownRecord(record.launch_context),
        model: asOptionalNullableString(record.model),
        llm_provider: asOptionalNullableString(record.llm_provider),
        llm_profile: asOptionalNullableString(record.llm_profile),
        reasoning_effort: asOptionalNullableString(record.reasoning_effort),
        run_id: asOptionalNullableString(record.run_id),
        launch_error: asOptionalNullableString(record.launch_error),
        review_message: asOptionalNullableString(record.review_message),
    }
}

function parseFlowLaunchResponse(value: unknown): FlowLaunchResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.created_at !== 'string'
        || typeof record.updated_at !== 'string'
        || typeof record.flow_name !== 'string'
        || typeof record.summary !== 'string'
        || typeof record.project_path !== 'string'
        || typeof record.conversation_id !== 'string'
        || typeof record.source_turn_id !== 'string'
    ) {
        return null
    }
    const status = record.status === 'launch_failed'
        || record.status === 'launched'
        ? record.status
        : 'pending'
    return {
        id: record.id,
        created_at: record.created_at,
        updated_at: record.updated_at,
        flow_name: record.flow_name,
        summary: record.summary,
        project_path: record.project_path,
        conversation_id: record.conversation_id,
        source_turn_id: record.source_turn_id,
        status,
        source_segment_id: asOptionalNullableString(record.source_segment_id),
        goal: asOptionalNullableString(record.goal),
        launch_context: asUnknownRecord(record.launch_context),
        model: asOptionalNullableString(record.model),
        llm_provider: asOptionalNullableString(record.llm_provider),
        llm_profile: asOptionalNullableString(record.llm_profile),
        reasoning_effort: asOptionalNullableString(record.reasoning_effort),
        run_id: asOptionalNullableString(record.run_id),
        launch_error: asOptionalNullableString(record.launch_error),
    }
}

function parseProposedPlanArtifactResponse(value: unknown): ProposedPlanArtifactResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.created_at !== 'string'
        || typeof record.updated_at !== 'string'
        || typeof record.title !== 'string'
        || typeof record.content !== 'string'
        || typeof record.project_path !== 'string'
        || typeof record.conversation_id !== 'string'
        || typeof record.source_turn_id !== 'string'
    ) {
        return null
    }
    const status = record.status === 'approved'
        || record.status === 'rejected'
        || record.status === 'launch_failed'
        ? record.status
        : 'pending_review'
    return {
        id: record.id,
        created_at: record.created_at,
        updated_at: record.updated_at,
        title: record.title,
        content: record.content,
        project_path: record.project_path,
        conversation_id: record.conversation_id,
        source_turn_id: record.source_turn_id,
        status,
        source_segment_id: asOptionalNullableString(record.source_segment_id),
        review_note: asOptionalNullableString(record.review_note),
        written_change_request_path: asOptionalNullableString(record.written_change_request_path),
        flow_launch_id: asOptionalNullableString(record.flow_launch_id),
        run_id: asOptionalNullableString(record.run_id),
        launch_error: asOptionalNullableString(record.launch_error),
    }
}

function parseConversationRowPayload(value: unknown, endpoint: string): ConversationRowPayload {
    const record = expectObjectRecord(value, endpoint)
    const metadata = asUnknownRecord(record.metadata) ?? {}
    const effective = asUnknownRecord(record.effective) ?? {}
    const field = (key: string) => asOptionalNullableString(effective[key]) ?? asOptionalNullableString(metadata[key]) ?? null
    const modelSettingsView = record.settings == null ? undefined : parseModelSettingsView(record.settings, endpoint)
    const settingsRevision = typeof metadata.settings_revision === 'number' ? String(metadata.settings_revision) : metadata.settings_revision
    return {
        handle: asOptionalNullableString(record.handle) ?? null,
        project_path: expectString(record.project_path, endpoint, 'project_path'),
        title: asOptionalString(record.title) || 'New thread',
        created_at: asOptionalString(record.created_at) || '',
        updated_at: asOptionalString(record.updated_at) || asOptionalString(record.created_at) || '',
        preview: asOptionalNullableString(record.preview) || null,
        running: record.running === true,
        needs_attention: record.needs_attention === true,
        launched_run_ids: Array.isArray(record.launched_run_ids)
            ? record.launched_run_ids.filter((id): id is string => typeof id === 'string')
            : [],
        chat_mode: metadata.chat_mode === 'plan' ? 'plan' : 'chat',
        provider: field('provider') || 'codex',
        model: field('model'),
        reasoning_effort: field('reasoning_effort'),
        model_settings_view: modelSettingsView,
        settings_revision: modelSettingsView?.revision
            ?? (settingsRevision == null ? null : expectDecimal(settingsRevision, endpoint, 'metadata.settings_revision')),
        ...('standing_role' in record ? { standing_role: parseStandingRole(record.standing_role) } : {}),
        placeholder: record.placeholder === true,
    }
}

const ARTIFACT_PARSERS: Record<ConversationArtifactCollection, (value: unknown) => unknown> = {
    flow_run_requests: parseFlowRunRequestResponse,
    flow_launches: parseFlowLaunchResponse,
    proposed_plans: parseProposedPlanArtifactResponse,
    event_log: parseWorkflowEventResponse,
    run_recoveries: asUnknownRecord,
}

function parseConversationRow(value: unknown, endpoint: string): ConversationRow {
    const record = expectObjectRecord(value, endpoint)
    const kind = record.kind
    const deleted = record.deleted
    if (typeof deleted !== 'boolean') {
        throw new ApiSchemaError(endpoint, 'Expected row "deleted" to be a boolean.')
    }
    const base = {
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        id: expectString(record.id, endpoint, 'id'),
        revision: expectDecimal(record.revision, endpoint, 'revision'),
        deleted,
        ...(record.turn_id == null ? {} : { turn_id: expectString(record.turn_id, endpoint, 'turn_id') }),
        ...(record.position == null ? {} : { position: expectDecimal(record.position, endpoint, 'position') }),
        ...(record.order == null ? {} : { order: expectDecimal(record.order, endpoint, 'order') }),
    }
    const invalid = (what: string): never => {
        throw new ApiSchemaError(endpoint, `Invalid ${what} row ${base.id}.`)
    }
    // Every upsert carries its whole entity; a payload that fails validation fails the whole response.
    const payload = <T>(parse: (value: unknown) => T | null): T | null => (
        deleted ? null : parse(record.payload) ?? invalid(String(kind))
    )
    switch (kind) {
    case 'conversation':
        if (base.id !== base.conversation_id) invalid(kind)
        return { ...base, kind, payload: payload((entry) => parseConversationRowPayload(entry, endpoint)) }
    case 'turn': {
        if (!deleted && base.position === undefined) invalid(kind)
        const turn = payload(parseConversationTurnResponse)
        if (turn && turn.id !== base.id) invalid(kind)
        return { ...base, kind, payload: turn }
    }
    case 'segment': {
        if (base.turn_id === undefined || (!deleted && base.order === undefined)) invalid(kind)
        const segment = payload(parseConversationSegmentResponse)
        if (segment && (segment.id !== base.id || segment.turn_id !== base.turn_id)) invalid(kind)
        return { ...base, kind, payload: segment }
    }
    case 'attachment':
        return { ...base, kind, payload: payload(parseConversationAttachment) }
    case 'artifact': {
        const collection = record.collection as ConversationArtifactCollection
        const parse = typeof collection === 'string' ? ARTIFACT_PARSERS[collection] : undefined
        if (!parse) invalid(kind)
        return { ...base, kind, collection, payload: payload(parse!) as Extract<ConversationRow, { kind: 'artifact' }>['payload'] }
    }
    default:
        throw new ApiSchemaError(endpoint, `Unknown row kind ${String(kind)}.`)
    }
}

function parseRows(value: unknown, endpoint: string, conversationId?: string): ConversationRow[] {
    if (!Array.isArray(value)) {
        throw new ApiSchemaError(endpoint, 'Expected "rows" to be an array.')
    }
    return value.map((entry) => {
        const row = parseConversationRow(entry, endpoint)
        if (conversationId !== undefined && row.conversation_id !== conversationId) {
            throw new ApiSchemaError(endpoint, `Row ${row.id} belongs to another conversation.`)
        }
        return row
    })
}

function expectAtOrBelow(rows: ConversationRow[], watermark: string, endpoint: string) {
    if (rows.some((row) => compareRevisions(row.revision, watermark) > 0)) {
        throw new ApiSchemaError(endpoint, 'A row is newer than its response watermark.')
    }
}

function expectStringArray(value: unknown, endpoint: string, field: string): string[] {
    if (!Array.isArray(value) || !value.every((entry) => typeof entry === 'string')) {
        throw new ApiSchemaError(endpoint, `Expected "${field}" to be an array of strings.`)
    }
    return value
}

/**
 * A revision batch, validated whole. A catch-up batch holds exactly the changes after
 * `from_revision`; a mutation result only promises nothing past its watermark.
 */
export function parseConversationBatchResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}/changes',
    kind: 'catch_up' | 'mutation' = 'catch_up',
): ConversationBatch {
    const record = expectObjectRecord(payload, endpoint)
    const generation = expectString(record.generation, endpoint, 'generation')
    const from_revision = expectDecimal(record.from_revision, endpoint, 'from_revision')
    const watermark = expectDecimal(record.watermark, endpoint, 'watermark')
    if (compareRevisions(from_revision, watermark) > 0) {
        throw new ApiSchemaError(endpoint, 'Batch starts after its watermark.')
    }
    const rows = parseRows(record.rows, endpoint)
    expectAtOrBelow(rows, watermark, endpoint)
    if (kind === 'catch_up' && rows.some((row) => compareRevisions(row.revision, from_revision) <= 0)) {
        throw new ApiSchemaError(endpoint, 'A catch-up row is not after the batch start.')
    }
    return { generation, from_revision, watermark, rows }
}

export function parseConversationPageResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}',
): ConversationPage {
    const record = expectObjectRecord(payload, endpoint)
    const conversation_id = expectString(record.conversation_id, endpoint, 'conversation_id')
    const watermark = expectDecimal(record.watermark, endpoint, 'watermark')
    const rows = parseRows(record.rows, endpoint, conversation_id)
    expectAtOrBelow(rows, watermark, endpoint)
    if (!Array.isArray(record.pinned_artifact_ids)) {
        throw new ApiSchemaError(endpoint, 'Expected "pinned_artifact_ids" to be an array.')
    }
    return {
        generation: expectString(record.generation, endpoint, 'generation'),
        watermark,
        conversation_id,
        rows,
        turn_ids: expectStringArray(record.turn_ids, endpoint, 'turn_ids'),
        pinned_turn_ids: expectStringArray(record.pinned_turn_ids, endpoint, 'pinned_turn_ids'),
        pinned_artifact_ids: record.pinned_artifact_ids.map((entry) => {
            const pinned = expectObjectRecord(entry, endpoint)
            return {
                collection: expectString(pinned.collection, endpoint, 'collection'),
                id: expectString(pinned.id, endpoint, 'id'),
            }
        }),
        older_cursor: record.older_cursor == null ? null : expectString(record.older_cursor, endpoint, 'older_cursor'),
    }
}

export function parseConversationSummarySnapshotResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/conversations',
): ConversationSummarySnapshot {
    const record = expectObjectRecord(payload, endpoint)
    const watermark = expectDecimal(record.watermark, endpoint, 'watermark')
    const rows = parseRows(record.rows, endpoint)
    expectAtOrBelow(rows, watermark, endpoint)
    if (rows.some((row) => row.kind !== 'conversation')) {
        throw new ApiSchemaError(endpoint, 'A summary snapshot holds conversation rows only.')
    }
    return {
        generation: expectString(record.generation, endpoint, 'generation'),
        watermark,
        project_path: record.project_path == null ? null : expectString(record.project_path, endpoint, 'project_path'),
        rows,
    }
}

export function parseConversationMutationAckResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}',
): ConversationMutationAck {
    const record = expectObjectRecord(payload, endpoint)
    return {
        generation: expectString(record.generation, endpoint, 'generation'),
        revision: expectDecimal(record.revision, endpoint, 'revision'),
        changes: parseConversationBatchResponse(record.changes, endpoint, 'mutation'),
    }
}

export function parseConversationAcceptanceResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}/turns',
): ConversationAcceptance {
    const record = expectObjectRecord(payload, endpoint)
    const status = record.status
    if (status !== 'accepted' && status !== 'queued' && status !== 'delivery_pending') {
        throw new ApiSchemaError(endpoint, 'Invalid acceptance status.')
    }
    const user_turn_id = expectString(record.user_turn_id, endpoint, 'user_turn_id')
    return {
        ...parseConversationMutationAckResponse(record, endpoint),
        user_turn_id,
        client_turn_id: record.client_turn_id == null ? user_turn_id : expectString(record.client_turn_id, endpoint, 'client_turn_id'),
        assistant_turn_id: record.assistant_turn_id == null ? null : expectString(record.assistant_turn_id, endpoint, 'assistant_turn_id'),
        status,
    }
}

const RESET_CODES = ['window_reset', 'resync_required'] as const
const RESET_REASONS = ['order_changed', 'generation_changed', 'cursor_ahead', 'catchup_limit'] as const

export function parseConversationResetResponse(
    payload: unknown,
    endpoint = '/workspace/api/live/events',
): ConversationReset {
    const record = expectObjectRecord(payload, endpoint)
    const code = RESET_CODES.find((entry) => entry === record.code)
    const reason = RESET_REASONS.find((entry) => entry === record.reason)
    if (!code || !reason) {
        throw new ApiSchemaError(endpoint, 'Invalid reset code or reason.')
    }
    return {
        code,
        reason,
        generation: expectString(record.generation, endpoint, 'generation'),
        watermark: expectDecimal(record.watermark, endpoint, 'watermark'),
    }
}

/** The thread-list entry for a conversation row; summary-scope rows carry the standing role. */
export function conversationSummaryFromRow(
    row: Extract<ConversationRow, { kind: 'conversation' }> & { payload: ConversationRowPayload },
): ConversationSummaryResponse {
    const { payload } = row
    return {
        conversation_id: row.id,
        conversation_handle: payload.handle,
        project_path: payload.project_path,
        title: payload.title,
        created_at: payload.created_at,
        updated_at: payload.updated_at,
        revision: row.revision,
        last_message_preview: payload.preview || undefined,
        launched_run_ids: payload.launched_run_ids,
        standing_role: payload.standing_role ?? null,
        running: payload.running,
    }
}

export function parseConversationDeleteResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}',
): ConversationDeleteResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status') === 'deleted' ? 'deleted' : 'deleted',
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        conversation_handle: asOptionalNullableString(record.conversation_handle),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
    }
}

export function parseConversationStreamDeltaEventResponse(
    payload: unknown,
): ConversationStreamDeltaEventResponse | null {
    const record = asUnknownRecord(payload)
    if (
        !record
        || record.type !== 'stream_delta'
        || typeof record.conversation_id !== 'string'
        || typeof record.turn_id !== 'string'
        || typeof record.stream_sequence !== 'number'
        || !Number.isFinite(record.stream_sequence)
        || typeof record.base_revision !== 'number'
        || !Number.isFinite(record.base_revision)
    ) {
        return null
    }
    const base = {
        type: 'stream_delta' as const,
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
        stream_sequence: record.stream_sequence,
        base_revision: record.base_revision,
    }
    if (record.delta_kind === 'segment_text') {
        return typeof record.segment_id === 'string' && typeof record.text === 'string'
            ? { ...base, delta_kind: 'segment_text', segment_id: record.segment_id, text: record.text }
            : null
    }
    if (record.delta_kind === 'thinking') {
        return typeof record.active === 'boolean'
            ? { ...base, delta_kind: 'thinking', active: record.active, estimated_tokens: typeof record.estimated_tokens === 'number' ? record.estimated_tokens : null, elapsed_ms: typeof record.elapsed_ms === 'number' ? record.elapsed_ms : null }
            : null
    }
    return null
}

/** A conversation read whose 409 carries a Reset rather than an error detail. */
async function fetchConversationRead<T>(
    path: string,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    const response = await fetch(workspaceUrl(path))
    if (response.status === 409) {
        const reset = await response.clone().json()
            .then((payload: unknown) => parseConversationResetResponse(payload, endpoint))
            .catch(() => null)
        if (reset) {
            throw new ConversationResetError(endpoint, reset)
        }
    }
    if (!response.ok) {
        throw await extractHttpError(response, endpoint)
    }
    return parser(await parseJsonPayload(response, endpoint), endpoint)
}

/** Every project's summaries without a project path. */
export async function fetchProjectConversationListValidated(
    projectPath: string | null,
): Promise<ConversationSummarySnapshot> {
    return fetchWorkspaceJsonValidated(
        `/projects/conversations${projectPath === null ? '' : `?project_path=${encodeURIComponent(projectPath)}`}`,
        undefined,
        '/workspace/api/projects/conversations',
        parseConversationSummarySnapshotResponse,
    )
}

/** The newest page, or the page before `before`; a stale cursor throws ConversationResetError. */
export async function fetchConversationPageValidated(
    conversationId: string,
    projectPath: string,
    before: string | null = null,
): Promise<ConversationPage> {
    return fetchConversationRead(
        `/conversations/${encodeURIComponent(conversationId)}?project_path=${encodeURIComponent(projectPath)}&limit=50${before === null ? '' : `&before=${encodeURIComponent(before)}`}`,
        '/workspace/api/conversations/{id}',
        parseConversationPageResponse,
    )
}

export async function fetchConversationChangesValidated(
    conversationId: string,
    projectPath: string,
    generation: string,
    since: string,
): Promise<ConversationBatch> {
    return fetchConversationRead(
        `/conversations/${encodeURIComponent(conversationId)}/changes?project_path=${encodeURIComponent(projectPath)}&generation=${encodeURIComponent(generation)}&since=${encodeURIComponent(since)}`,
        '/workspace/api/conversations/{id}/changes',
        (payload, endpoint) => parseConversationBatchResponse(payload, endpoint),
    )
}

export async function fetchConversationSegmentToolOutputValidated(
    conversationId: string,
    segmentId: string,
    projectPath: string,
): Promise<ConversationToolOutputResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/tool-output/${encodeURIComponent(segmentId)}?project_path=${encodeURIComponent(projectPath)}`,
        undefined,
        '/workspace/api/conversations/{id}/tool-output/{segmentId}',
        (payload) => {
            const endpoint = '/workspace/api/conversations/{id}/tool-output/{segmentId}'
            const record = expectObjectRecord(payload, endpoint)
            const output = expectString(record.output, endpoint, 'output')
            return {
                output,
                output_size: typeof record.output_size === 'number' ? record.output_size : output.length,
            }
        },
    )
}

export async function deleteConversationValidated(
    conversationId: string,
    projectPath: string,
): Promise<ConversationDeleteResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}?project_path=${encodeURIComponent(projectPath)}`,
        {
            method: 'DELETE',
        },
        '/workspace/api/conversations/{id}',
        parseConversationDeleteResponse,
    )
}

export function conversationAttachmentUrl(conversationId: string, attachmentId: string, projectPath: string): string {
    return workspaceUrl(
        `/conversations/${encodeURIComponent(conversationId)}/attachments/${encodeURIComponent(attachmentId)}?project_path=${encodeURIComponent(projectPath)}`,
    )
}

/** Uploads the raw file; the server detects the media type and assigns the id. */
export async function uploadConversationAttachmentValidated(
    conversationId: string,
    projectPath: string,
    file: File,
): Promise<ConversationAttachmentResponse> {
    const endpoint = '/workspace/api/conversations/{id}/attachments'
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/attachments?project_path=${encodeURIComponent(projectPath)}&filename=${encodeURIComponent(file.name)}`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/octet-stream' },
            body: file,
        },
        endpoint,
        (payload) => {
            const attachment = parseConversationAttachment(payload)
            if (!attachment) {
                throw new ApiSchemaError(endpoint, 'Invalid attachment response.')
            }
            return attachment
        },
    )
}

export async function deleteConversationAttachmentValidated(
    conversationId: string,
    attachmentId: string,
    projectPath: string,
): Promise<void> {
    await fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/attachments/${encodeURIComponent(attachmentId)}?project_path=${encodeURIComponent(projectPath)}`,
        { method: 'DELETE' },
        '/workspace/api/conversations/{id}/attachments/{attachmentId}',
        () => undefined,
    )
}

export async function sendConversationTurnValidated(
    conversationId: string,
    payload: {
        project_path: string
        message: string
        client_turn_id?: string
        attachment_ids?: string[]
        provider?: string | null
        llm_profile?: string | null
        model?: string | null
        reasoning_effort?: string | null
        chat_mode?: ConversationChatMode | null
    },
): Promise<ConversationAcceptance> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/turns`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/turns',
        parseConversationAcceptanceResponse,
    )
}

/** Resends a failed assistant turn's message as a new turn. */
export async function retryConversationTurnValidated(
    conversationId: string,
    turnId: string,
    projectPath: string,
): Promise<ConversationAcceptance> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/turns/${encodeURIComponent(turnId)}/retry`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_path: projectPath }),
        },
        '/workspace/api/conversations/{id}/turns/{turnId}/retry',
        parseConversationAcceptanceResponse,
    )
}

export async function updateConversationSettingsValidated(
    conversationId: string,
    payload: {
        project_path: string
        expected_revision: string
        model_settings?: ModelSettings | null
        chat_mode?: ConversationChatMode | null
        provider?: string | null
        model?: string | null
        reasoning_effort?: string | null
    },
): Promise<ConversationMutationAck> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/settings`,
        {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ ...payload, model_settings: modelSettingsForApi(payload.model_settings) }),
        },
        '/workspace/api/conversations/{id}/settings',
        parseConversationMutationAckResponse,
    )
}

export async function submitConversationRequestUserInputValidated(
    conversationId: string,
    requestId: string,
    payload: {
        project_path: string
        answers: Record<string, string>
    },
): Promise<ConversationMutationAck> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/request-user-input/${encodeURIComponent(requestId)}/answer`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/request-user-input/{requestId}/answer',
        parseConversationMutationAckResponse,
    )
}

export async function reviewFlowRunRequestValidated(
    conversationId: string,
    requestId: string,
    payload: {
        project_path: string
        disposition: 'approved' | 'rejected'
        message: string
        flow_name?: string | null
        model?: string | null
        llm_provider?: string | null
        llm_profile?: string | null
        reasoning_effort?: string | null
    },
): Promise<ConversationMutationAck> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/flow-run-requests/${encodeURIComponent(requestId)}/review`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/flow-run-requests/{requestId}/review',
        parseConversationMutationAckResponse,
    )
}

export async function reviewProposedPlanValidated(
    conversationId: string,
    planId: string,
    payload: {
        project_path: string
        disposition: 'approved' | 'rejected'
        review_note?: string | null
    },
): Promise<ConversationMutationAck> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/proposed-plans/${encodeURIComponent(planId)}/review`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/proposed-plans/{planId}/review',
        parseConversationMutationAckResponse,
    )
}

export async function interruptConversationTurnValidated(conversationId: string, projectPath: string): Promise<boolean> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/interrupt`,
        { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ project_path: projectPath }) },
        '/workspace/api/conversations/{id}/interrupt',
        (value) => {
            const record = asUnknownRecord(value)
            if (typeof record?.interrupted !== 'boolean') throw new Error('Invalid interrupt response')
            return record.interrupted
        },
    )
}
