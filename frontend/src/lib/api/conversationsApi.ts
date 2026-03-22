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
    kind: 'message' | 'spec_edit_proposal' | 'flow_run_request' | 'flow_launch' | 'execution_card'
    artifact_id?: string | null
    parent_turn_id?: string | null
    error?: string | null
}

export interface ConversationSegmentResponse {
    id: string
    turn_id: string
    order: number
    kind: 'assistant_message' | 'reasoning' | 'tool_call' | 'spec_edit_proposal' | 'flow_run_request' | 'flow_launch' | 'execution_card'
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

export interface SpecEditProposalResponse {
    id: string
    created_at: string
    summary: string
    status: 'pending' | 'applied' | 'rejected'
    changes: Array<{
        path: string
        before: string
        after: string
    }>
    canonical_spec_edit_id?: string | null
    approved_at?: string | null
    git_branch?: string | null
    git_commit?: string | null
}

export interface ExecutionCardResponse {
    id: string
    title: string
    summary: string
    objective: string
    status: 'draft' | 'approved' | 'rejected' | 'revision-requested'
    source_spec_edit_id: string
    source_workflow_run_id: string
    created_at: string
    updated_at: string
    flow_source?: string | null
    work_items: Array<{
        id: string
        title: string
        description: string
        acceptance_criteria: string[]
        depends_on: string[]
    }>
    review_feedback: Array<{
        id: string
        disposition: 'approved' | 'rejected' | 'revision_requested'
        message: string
        created_at: string
        author: string
    }>
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
    title: string
    created_at: string
    updated_at: string
    turns: ConversationTurnResponse[]
    segments: ConversationSegmentResponse[]
    event_log: WorkflowEventResponse[]
    spec_edit_proposals: SpecEditProposalResponse[]
    flow_run_requests: FlowRunRequestResponse[]
    flow_launches: FlowLaunchResponse[]
    execution_cards: ExecutionCardResponse[]
    execution_workflow: {
        run_id?: string | null
        status: 'idle' | 'running' | 'failed'
        error?: string | null
        flow_source?: string | null
    }
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

function parseConversationTurnResponse(value: unknown, _endpoint: string): ConversationTurnResponse | null {
    const record = asUnknownRecord(value)
    if (!record) {
        return null
    }
    const role = typeof record.role === 'string' ? record.role : ''
    if (role !== 'user' && role !== 'assistant' && role !== 'system') {
        return null
    }
    const kind = record.kind === 'spec_edit_proposal' || record.kind === 'flow_run_request' || record.kind === 'flow_launch' || record.kind === 'execution_card' || record.kind === 'message'
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
        || record.kind === 'spec_edit_proposal'
        || record.kind === 'flow_run_request'
        || record.kind === 'flow_launch'
        || record.kind === 'execution_card'
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

function parseSpecEditProposalResponse(value: unknown): SpecEditProposalResponse | null {
    const record = asUnknownRecord(value)
    if (!record || typeof record.id !== 'string' || typeof record.created_at !== 'string' || typeof record.summary !== 'string') {
        return null
    }
    const rawChanges = record.changes
    const changes = Array.isArray(rawChanges)
        ? rawChanges
            .map((change) => asUnknownRecord(change))
            .filter((change): change is Record<string, unknown> => change !== null)
            .filter((change) => typeof change.path === 'string' && typeof change.before === 'string' && typeof change.after === 'string')
            .map((change) => ({
                path: String(change.path),
                before: String(change.before),
                after: String(change.after),
            }))
        : []
    const status = record.status === 'applied' || record.status === 'rejected' ? record.status : 'pending'
    return {
        id: record.id,
        created_at: record.created_at,
        summary: record.summary,
        status,
        changes,
        canonical_spec_edit_id: asOptionalNullableString(record.canonical_spec_edit_id),
        approved_at: asOptionalNullableString(record.approved_at),
        git_branch: asOptionalNullableString(record.git_branch),
        git_commit: asOptionalNullableString(record.git_commit),
    }
}

function parseExecutionCardResponse(value: unknown): ExecutionCardResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.title !== 'string'
        || typeof record.summary !== 'string'
        || typeof record.objective !== 'string'
        || typeof record.source_spec_edit_id !== 'string'
        || typeof record.source_workflow_run_id !== 'string'
        || typeof record.created_at !== 'string'
        || typeof record.updated_at !== 'string'
    ) {
        return null
    }
    const status = record.status === 'approved'
        || record.status === 'rejected'
        || record.status === 'revision-requested'
        ? record.status
        : 'draft'
    const work_items = Array.isArray(record.work_items)
        ? record.work_items
            .map((item) => asUnknownRecord(item))
            .filter((item): item is Record<string, unknown> => item !== null)
            .filter((item) => typeof item.id === 'string' && typeof item.title === 'string' && typeof item.description === 'string')
            .map((item) => ({
                id: String(item.id),
                title: String(item.title),
                description: String(item.description),
                acceptance_criteria: Array.isArray(item.acceptance_criteria) ? item.acceptance_criteria.map((entry) => String(entry)) : [],
                depends_on: Array.isArray(item.depends_on) ? item.depends_on.map((entry) => String(entry)) : [],
            }))
        : []
    const review_feedback = Array.isArray(record.review_feedback)
        ? record.review_feedback
            .map((entry) => asUnknownRecord(entry))
            .filter((entry): entry is Record<string, unknown> => entry !== null)
            .filter((entry) => typeof entry.id === 'string' && typeof entry.disposition === 'string' && typeof entry.message === 'string' && typeof entry.created_at === 'string')
            .map((entry): ExecutionCardResponse['review_feedback'][number] => ({
                id: String(entry.id),
                disposition: entry.disposition === 'approved' || entry.disposition === 'rejected' || entry.disposition === 'revision_requested'
                    ? entry.disposition
                    : 'revision_requested',
                message: String(entry.message),
                created_at: String(entry.created_at),
                author: typeof entry.author === 'string' ? entry.author : 'user',
            }))
        : []
    return {
        id: record.id,
        title: record.title,
        summary: record.summary,
        objective: record.objective,
        status,
        source_spec_edit_id: record.source_spec_edit_id,
        source_workflow_run_id: record.source_workflow_run_id,
        created_at: record.created_at,
        updated_at: record.updated_at,
        flow_source: asOptionalNullableString(record.flow_source),
        work_items,
        review_feedback,
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

function parseConversationSummaryResponse(value: unknown, _endpoint: string): ConversationSummaryResponse | null {
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
        .map((entry) => parseConversationSummaryResponse(entry, endpoint))
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
            .map((entry) => parseConversationTurnResponse(entry, endpoint))
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
    const spec_edit_proposals = Array.isArray(record.spec_edit_proposals)
        ? record.spec_edit_proposals
            .map((entry) => parseSpecEditProposalResponse(entry))
            .filter((entry): entry is SpecEditProposalResponse => entry !== null)
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
    const execution_cards = Array.isArray(record.execution_cards)
        ? record.execution_cards
            .map((entry) => parseExecutionCardResponse(entry))
            .filter((entry): entry is ExecutionCardResponse => entry !== null)
        : []
    const executionWorkflowRecord = asUnknownRecord(record.execution_workflow) || {}
    return {
        schema_version: schemaVersion,
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        conversation_handle: asOptionalNullableString(record.conversation_handle),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
        title: asOptionalString(record.title) || 'New thread',
        created_at: asOptionalString(record.created_at) || '',
        updated_at: asOptionalString(record.updated_at) || asOptionalString(record.created_at) || '',
        turns,
        segments,
        event_log,
        spec_edit_proposals,
        flow_run_requests,
        flow_launches,
        execution_cards,
        execution_workflow: {
            run_id: asOptionalNullableString(executionWorkflowRecord.run_id),
            status: executionWorkflowRecord.status === 'running' || executionWorkflowRecord.status === 'failed'
                ? executionWorkflowRecord.status
                : 'idle',
            error: asOptionalNullableString(executionWorkflowRecord.error),
            flow_source: asOptionalNullableString(executionWorkflowRecord.flow_source),
        },
    }
}

export function parseConversationStreamEventResponse(
    payload: unknown,
    endpoint = '/workspace/api/conversations/{id}/events',
): ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse | null {
    const record = expectObjectRecord(payload, endpoint)
    const type = typeof record.type === 'string' ? record.type : ''
    if (type === 'turn_upsert') {
        const turn = parseConversationTurnResponse(record.turn, endpoint)
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
    payload: { project_path: string; message: string; model?: string | null },
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

export async function approveSpecEditProposalValidated(
    conversationId: string,
    proposalId: string,
    payload: { project_path: string; model?: string | null; flow_source?: string | null },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/spec-edit-proposals/${encodeURIComponent(proposalId)}/approve`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/spec-edit-proposals/{proposalId}/approve',
        parseConversationSnapshotResponse,
    )
}

export async function rejectSpecEditProposalValidated(
    conversationId: string,
    proposalId: string,
    payload: { project_path: string },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/spec-edit-proposals/${encodeURIComponent(proposalId)}/reject`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/spec-edit-proposals/{proposalId}/reject',
        parseConversationSnapshotResponse,
    )
}

export async function reviewExecutionCardValidated(
    conversationId: string,
    executionCardId: string,
    payload: {
        project_path: string
        disposition: 'approved' | 'rejected' | 'revision_requested'
        message: string
        model?: string | null
        flow_source?: string | null
    },
): Promise<ConversationSnapshotResponse> {
    return fetchWorkspaceJsonValidated(
        `/conversations/${encodeURIComponent(conversationId)}/execution-cards/${encodeURIComponent(executionCardId)}/review`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/conversations/{id}/execution-cards/{executionCardId}/review',
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
