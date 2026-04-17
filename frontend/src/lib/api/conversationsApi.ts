import {
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated, workspaceUrl } from './apiClient'

export interface ConversationTurnResponse {
    id: string
    role: 'user' | 'assistant' | 'system'
    content: string
    timestamp: string
    status: 'pending' | 'streaming' | 'complete' | 'failed'
    kind: 'message' | 'mode_change' | 'flow_run_request' | 'flow_launch'
    artifact_id?: string | null
    parent_turn_id?: string | null
    error?: string | null
}

export type ConversationChatMode = 'chat' | 'plan'

export interface ConversationSegmentResponse {
    id: string
    turn_id: string
    order: number
    kind: 'assistant_message' | 'reasoning' | 'tool_call' | 'flow_run_request' | 'flow_launch'
    role: 'assistant' | 'system'
    status: 'pending' | 'streaming' | 'complete' | 'failed' | 'running'
    timestamp: string
    updated_at: string
    content: string
    completed_at?: string | null
    error?: string | null
    artifact_id?: string | null
    phase?: string | null
    tool_call?: {
        kind: 'command_execution' | 'file_change' | 'dynamic_tool'
        status: 'running' | 'completed' | 'failed'
        id: string
        title: string
        command?: string | null
        output?: string | null
        file_paths: string[]
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
    run_id?: string | null
    launch_error?: string | null
}

export interface ConversationSnapshotResponse {
    schema_version: number
    conversation_id: string
    conversation_handle?: string | null
    project_path: string
    chat_mode: ConversationChatMode
    title: string
    created_at: string
    updated_at: string
    turns: ConversationTurnResponse[]
    segments: ConversationSegmentResponse[]
    event_log: WorkflowEventResponse[]
    flow_run_requests: FlowRunRequestResponse[]
    flow_launches: FlowLaunchResponse[]
}

export interface ConversationSummaryResponse {
    conversation_id: string
    conversation_handle?: string | null
    project_path: string
    title: string
    created_at: string
    updated_at: string
    last_message_preview?: string | null
}

export interface ConversationDeleteResponse {
    status: 'deleted'
    conversation_id: string
    conversation_handle?: string | null
    project_path: string
}

export interface ConversationTurnUpsertEventResponse {
    type: 'turn_upsert'
    conversation_id: string
    project_path: string
    title: string
    updated_at: string
    turn: ConversationTurnResponse
}

export interface ConversationSegmentUpsertEventResponse {
    type: 'segment_upsert'
    conversation_id: string
    project_path: string
    title: string
    updated_at: string
    segment: ConversationSegmentResponse
}

export function conversationEventsUrl(conversationId: string, projectPath: string): string {
    return workspaceUrl(
        `/conversations/${encodeURIComponent(conversationId)}/events?project_path=${encodeURIComponent(projectPath)}`,
    )
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
        || record.kind === 'message'
        ? record.kind
        : 'message'
    if (typeof record.id !== 'string' || typeof record.content !== 'string' || typeof record.timestamp !== 'string') {
        return null
    }
    const status = record.status === 'pending'
        || record.status === 'streaming'
        || record.status === 'failed'
        ? record.status
        : 'complete'
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
    }
}

function parseConversationSegmentResponse(value: unknown): ConversationSegmentResponse | null {
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
        || record.kind === 'reasoning'
        || record.kind === 'tool_call'
        || record.kind === 'flow_run_request'
        || record.kind === 'flow_launch'
        ? record.kind
        : null
    if (!kind) {
        return null
    }
    const role = record.role === 'system' ? 'system' : 'assistant'
    const status = record.status === 'pending'
        || record.status === 'streaming'
        || record.status === 'failed'
        || record.status === 'running'
        ? record.status
        : 'complete'
    const toolCall = asUnknownRecord(record.tool_call)
    const source = asUnknownRecord(record.source)
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
        tool_call: toolCall && typeof toolCall.title === 'string' && typeof toolCall.id === 'string'
            ? {
                id: toolCall.id,
                kind: toolCall.kind === 'file_change'
                    ? 'file_change'
                    : toolCall.kind === 'dynamic_tool'
                        ? 'dynamic_tool'
                        : 'command_execution',
                status: toolCall.status === 'running' || toolCall.status === 'failed' ? toolCall.status : 'completed',
                title: toolCall.title,
                command: asOptionalNullableString(toolCall.command),
                output: asOptionalNullableString(toolCall.output),
                file_paths: Array.isArray(toolCall.file_paths) ? toolCall.file_paths.map((entry) => String(entry)) : [],
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
    return {
        message: record.message,
        timestamp: record.timestamp,
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
        run_id: asOptionalNullableString(record.run_id),
        launch_error: asOptionalNullableString(record.launch_error),
    }
}

function parseConversationSummaryResponse(value: unknown): ConversationSummaryResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.conversation_id !== 'string'
        || typeof record.project_path !== 'string'
        || typeof record.title !== 'string'
        || typeof record.created_at !== 'string'
        || typeof record.updated_at !== 'string'
    ) {
        return null
    }
    return {
        conversation_id: record.conversation_id,
        conversation_handle: asOptionalNullableString(record.conversation_handle),
        project_path: record.project_path,
        title: record.title,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_message_preview: asOptionalNullableString(record.last_message_preview),
    }
}

export function parseConversationSummaryListResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/conversations',
): ConversationSummaryResponse[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of conversation summaries.')
    }
    return payload
        .map((entry) => parseConversationSummaryResponse(entry))
        .filter((entry): entry is ConversationSummaryResponse => entry !== null)
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

export function parseConversationSnapshotResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}',
): ConversationSnapshotResponse {
    const record = expectObjectRecord(payload, endpoint)
    const schemaVersion = record.schema_version
    if (typeof schemaVersion !== 'number') {
        throw new ApiSchemaError(endpoint, 'Expected conversation snapshot schema_version.')
    }
    const turns = Array.isArray(record.turns)
        ? record.turns
            .map((entry) => parseConversationTurnResponse(entry))
            .filter((entry): entry is ConversationTurnResponse => entry !== null)
        : []
    const event_log = Array.isArray(record.event_log)
        ? record.event_log
            .map((entry) => parseWorkflowEventResponse(entry))
            .filter((entry): entry is WorkflowEventResponse => entry !== null)
        : []
    const segments = Array.isArray(record.segments)
        ? record.segments
            .map((entry) => parseConversationSegmentResponse(entry))
            .filter((entry): entry is ConversationSegmentResponse => entry !== null)
        : []
    const flow_run_requests = Array.isArray(record.flow_run_requests)
        ? record.flow_run_requests
            .map((entry) => parseFlowRunRequestResponse(entry))
            .filter((entry): entry is FlowRunRequestResponse => entry !== null)
        : []
    const flow_launches = Array.isArray(record.flow_launches)
        ? record.flow_launches
            .map((entry) => parseFlowLaunchResponse(entry))
            .filter((entry): entry is FlowLaunchResponse => entry !== null)
        : []
    return {
        schema_version: schemaVersion,
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        conversation_handle: asOptionalNullableString(record.conversation_handle),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
        chat_mode: record.chat_mode === 'plan' ? 'plan' : 'chat',
        title: asOptionalString(record.title) || 'New thread',
        created_at: asOptionalString(record.created_at) || '',
        updated_at: asOptionalString(record.updated_at) || asOptionalString(record.created_at) || '',
        turns,
        segments,
        event_log,
        flow_run_requests,
        flow_launches,
    }
}

export function parseConversationStreamEventResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}/events',
): ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse | null {
    const record = expectObjectRecord(payload, endpoint)
    const type = typeof record.type === 'string' ? record.type : ''
    if (type === 'turn_upsert') {
        const turn = parseConversationTurnResponse(record.turn)
        if (
            !turn
            || typeof record.conversation_id !== 'string'
            || typeof record.project_path !== 'string'
            || typeof record.title !== 'string'
            || typeof record.updated_at !== 'string'
        ) {
            return null
        }
        return {
            type: 'turn_upsert',
            conversation_id: record.conversation_id,
            project_path: record.project_path,
            title: record.title,
            updated_at: record.updated_at,
            turn,
        }
    }
    if (type === 'segment_upsert') {
        const segment = parseConversationSegmentResponse(record.segment)
        if (
            !segment
            || typeof record.conversation_id !== 'string'
            || typeof record.project_path !== 'string'
            || typeof record.title !== 'string'
            || typeof record.updated_at !== 'string'
        ) {
            return null
        }
        return {
            type: 'segment_upsert',
            conversation_id: record.conversation_id,
            project_path: record.project_path,
            title: record.title,
            updated_at: record.updated_at,
            segment,
        }
    }
    return null
}

export async function fetchProjectConversationListValidated(
    projectPath: string,
): Promise<ConversationSummaryResponse[]> {
    return fetchWorkspaceJsonValidated(
        `/projects/conversations?project_path=${encodeURIComponent(projectPath)}`,
        undefined,
        '/workspace/api/projects/conversations',
        parseConversationSummaryListResponse,
    )
}

export async function fetchConversationSnapshotValidated(
    conversationId: string,
    projectPath: string,
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}?project_path=${encodeURIComponent(projectPath)}`,
        undefined,
        '/workspace/api/conversations/{id}',
        parseConversationSnapshotResponse,
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

export async function sendConversationTurnValidated(
    conversationId: string,
    payload: { project_path: string; message: string; model?: string | null; chat_mode?: ConversationChatMode | null },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/turns`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/turns',
        parseConversationSnapshotResponse,
    )
}

export async function updateConversationSettingsValidated(
    conversationId: string,
    payload: { project_path: string; chat_mode: ConversationChatMode },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/settings`,
        {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/settings',
        parseConversationSnapshotResponse,
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
    },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/flow-run-requests/${encodeURIComponent(requestId)}/review`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/flow-run-requests/{requestId}/review',
        parseConversationSnapshotResponse,
    )
}
