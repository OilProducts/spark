import type { CanonicalPreviewGraphPayload } from '@/lib/canonicalFlowModel'
import type { DiagnosticEntry } from '@/store'

type UnknownRecord = Record<string, unknown>

export interface FlowPayloadResponse {
    name: string
    content: string
}

export interface PreviewResponsePayload {
    status: string
    graph?: CanonicalPreviewGraphPayload
    diagnostics?: DiagnosticEntry[]
    errors?: DiagnosticEntry[]
    error?: string
}

export interface PipelineStartResponse {
    status: string
    pipeline_id?: string
    run_id?: string
    working_directory?: string
    model?: string
    diagnostics?: DiagnosticEntry[]
    errors?: DiagnosticEntry[]
    error?: string
}

export interface PipelineStatusResponse {
    pipeline_id: string
    status: string
    flow_name?: string
    working_directory?: string
    model?: string
    last_error?: string | null
    completed_nodes?: string[]
    started_at?: string
    ended_at?: string | null
    result?: string | null
}

export interface PipelineCancelResponse {
    status: string
    pipeline_id: string
}

export interface PipelineCheckpointResponse {
    pipeline_id: string
    checkpoint: Record<string, unknown>
}

export interface PipelineContextResponse {
    pipeline_id: string
    context: Record<string, unknown>
}

export interface PipelineQuestionsResponse {
    questions: Array<Record<string, unknown>>
}

export interface PipelineAnswerResponse {
    status: string
    pipeline_id: string
    question_id: string
}

export type PipelineGraphResponse = string

export interface RunRecordResponse {
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

export interface RunsListResponse {
    runs: RunRecordResponse[]
}

export interface RuntimeStatusResponse {
    status: string
    last_error?: string | null
    last_working_directory?: string | null
    last_model?: string | null
    last_completed_nodes?: string[] | null
    last_flow_name?: string | null
    last_run_id?: string | null
}

export interface ConversationTurnResponse {
    id: string
    role: 'user' | 'assistant' | 'system'
    content: string
    timestamp: string
    status: 'pending' | 'streaming' | 'complete' | 'failed'
    kind: 'message' | 'spec_edit_proposal' | 'execution_card'
    artifact_id?: string | null
    parent_turn_id?: string | null
    error?: string | null
}

export interface ConversationTurnEventResponse {
    id: string
    turn_id: string
    sequence: number
    timestamp: string
    kind:
        | 'assistant_delta'
        | 'assistant_completed'
        | 'assistant_failed'
        | 'tool_call_started'
        | 'tool_call_updated'
        | 'tool_call_completed'
        | 'tool_call_failed'
        | 'retry_started'
    content_delta?: string | null
    message?: string | null
    tool_call_id?: string | null
    tool_call?: {
        kind: 'command_execution' | 'file_change'
        status: 'running' | 'completed' | 'failed'
        id: string
        title: string
        command?: string | null
        output?: string | null
        file_paths: string[]
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

export interface ConversationSnapshotResponse {
    conversation_id: string
    project_path: string
    title: string
    created_at: string
    updated_at: string
    turns: ConversationTurnResponse[]
    turn_events: ConversationTurnEventResponse[]
    event_log: WorkflowEventResponse[]
    spec_edit_proposals: SpecEditProposalResponse[]
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
    project_path: string
    title: string
    created_at: string
    updated_at: string
    last_message_preview?: string | null
}

export interface ConversationDeleteResponse {
    status: 'deleted'
    conversation_id: string
    project_path: string
}

export type ProjectDirectoryPickResponse =
    | {
        status: 'selected'
        directory_path: string
    }
    | {
        status: 'canceled'
    }

export interface ConversationTurnUpsertEventResponse {
    type: 'turn_upsert'
    conversation_id: string
    project_path: string
    title: string
    updated_at: string
    turn: ConversationTurnResponse
}

export interface ConversationTurnEventStreamResponse {
    type: 'turn_event'
    conversation_id: string
    project_path: string
    title: string
    updated_at: string
    event: ConversationTurnEventResponse
}

export class ApiSchemaError extends Error {
    endpoint: string

    constructor(endpoint: string, message: string) {
        super(`${endpoint}: ${message}`)
        this.name = 'ApiSchemaError'
        this.endpoint = endpoint
    }
}

export class ApiHttpError extends Error {
    endpoint: string
    status: number
    detail: string | null

    constructor(endpoint: string, status: number, detail: string | null) {
        const suffix = detail ? `: ${detail}` : ''
        super(`${endpoint} responded with HTTP ${status}${suffix}`)
        this.name = 'ApiHttpError'
        this.endpoint = endpoint
        this.status = status
        this.detail = detail
    }
}

function expectObjectRecord(value: unknown, endpoint: string): UnknownRecord {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        throw new ApiSchemaError(endpoint, 'Expected object response payload.')
    }
    return value as UnknownRecord
}

function expectString(value: unknown, endpoint: string, fieldName: string): string {
    if (typeof value !== 'string') {
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be a string.`)
    }
    return value
}

function asOptionalString(value: unknown): string | undefined {
    if (typeof value !== 'string') {
        return undefined
    }
    return value
}

function asOptionalNullableString(value: unknown): string | null | undefined {
    if (value === null) {
        return null
    }
    return asOptionalString(value)
}

function asOptionalStringArray(value: unknown): string[] | undefined {
    if (!Array.isArray(value)) {
        return undefined
    }
    if (!value.every((entry) => typeof entry === 'string')) {
        return undefined
    }
    return [...value]
}

function asUnknownRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    return value as Record<string, unknown>
}

function parseDiagnosticEntry(value: unknown, endpoint: string): DiagnosticEntry | null {
    const record = asUnknownRecord(value)
    if (!record) {
        return null
    }
    const severity = typeof record.severity === 'string' ? record.severity : null
    const message = typeof record.message === 'string' ? record.message : null
    const ruleId = typeof record.rule_id === 'string'
        ? record.rule_id
        : typeof record.rule === 'string'
            ? record.rule
            : null
    if (!severity || !message || !ruleId) {
        return null
    }
    if (severity !== 'error' && severity !== 'warning' && severity !== 'info') {
        throw new ApiSchemaError(endpoint, `Expected diagnostic severity to be error|warning|info; got "${severity}".`)
    }
    const edgeRaw = record.edge
    const edge = Array.isArray(edgeRaw)
        && edgeRaw.length === 2
        && typeof edgeRaw[0] === 'string'
        && typeof edgeRaw[1] === 'string'
        ? [edgeRaw[0], edgeRaw[1]] as [string, string]
        : undefined
    return {
        rule_id: ruleId,
        severity,
        message,
        line: typeof record.line === 'number' ? record.line : undefined,
        node_id: typeof record.node_id === 'string'
            ? record.node_id
            : typeof record.node === 'string'
                ? record.node
                : undefined,
        edge,
        fix: typeof record.fix === 'string' ? record.fix : undefined,
    }
}

function parseDiagnosticList(value: unknown, endpoint: string, fieldName: string): DiagnosticEntry[] | undefined {
    if (value === undefined) {
        return undefined
    }
    if (!Array.isArray(value)) {
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be an array when present.`)
    }
    return value
        .map((entry) => parseDiagnosticEntry(entry, endpoint))
        .filter((entry): entry is DiagnosticEntry => entry !== null)
}

function parseHttpErrorDetail(payload: unknown): string | null {
    const record = asUnknownRecord(payload)
    if (!record) {
        return null
    }
    if (typeof record.error === 'string' && record.error.trim().length > 0) {
        return record.error.trim()
    }
    if (typeof record.detail === 'string' && record.detail.trim().length > 0) {
        return record.detail.trim()
    }
    const detailRecord = asUnknownRecord(record.detail)
    if (detailRecord && typeof detailRecord.error === 'string' && detailRecord.error.trim().length > 0) {
        return detailRecord.error.trim()
    }
    return null
}

async function parseJsonPayload(response: Response, endpoint: string): Promise<unknown> {
    try {
        return await response.json()
    } catch {
        throw new ApiSchemaError(endpoint, 'Expected JSON response body.')
    }
}

async function extractHttpError(response: Response, endpoint: string): Promise<ApiHttpError> {
    let detail: string | null = null
    try {
        const bodyText = await response.text()
        if (bodyText.trim().length > 0) {
            try {
                const payload = JSON.parse(bodyText) as unknown
                detail = parseHttpErrorDetail(payload)
            } catch {
                detail = bodyText.trim()
            }
        }
    } catch {
        detail = null
    }
    return new ApiHttpError(endpoint, response.status, detail)
}

async function fetchJsonWithValidation<T>(
    url: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    const response = await fetch(url, init)
    if (!response.ok) {
        throw await extractHttpError(response, endpoint)
    }
    const payload = await parseJsonPayload(response, endpoint)
    return parser(payload, endpoint)
}

async function fetchTextWithValidation<T>(
    url: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    const response = await fetch(url, init)
    if (!response.ok) {
        throw await extractHttpError(response, endpoint)
    }
    const payload = await response.text()
    return parser(payload, endpoint)
}

export function parseFlowListResponse(payload: unknown, endpoint = '/api/flows'): string[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of flow names.')
    }
    if (!payload.every((entry) => typeof entry === 'string')) {
        throw new ApiSchemaError(endpoint, 'Expected every flow name to be a string.')
    }
    return [...payload]
}

export function parseFlowPayloadResponse(payload: unknown, endpoint = '/api/flows/{name}'): FlowPayloadResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        name: typeof record.name === 'string' ? record.name : '',
        content: expectString(record.content, endpoint, 'content'),
    }
}

export function parsePreviewResponse(payload: unknown, endpoint = '/preview'): PreviewResponsePayload {
    const record = expectObjectRecord(payload, endpoint)
    const status = typeof record.status === 'string' ? record.status : 'ok'
    const graphRecord = asUnknownRecord(record.graph)
    let graph: CanonicalPreviewGraphPayload | undefined
    if (graphRecord) {
        const nodes = graphRecord.nodes
        const edges = graphRecord.edges
        if (!Array.isArray(nodes) || !Array.isArray(edges)) {
            throw new ApiSchemaError(endpoint, 'Expected "graph.nodes" and "graph.edges" to be arrays.')
        }
        const normalizedNodes = nodes
            .map((node) => asUnknownRecord(node))
            .filter((node): node is Record<string, unknown> => node !== null)
        const normalizedEdges = edges
            .map((edge) => asUnknownRecord(edge))
            .filter((edge): edge is Record<string, unknown> => edge !== null)
        graph = {
            nodes: normalizedNodes,
            edges: normalizedEdges,
            graph_attrs: asUnknownRecord(graphRecord.graph_attrs),
            defaults: asUnknownRecord(graphRecord.defaults),
            subgraphs: Array.isArray(graphRecord.subgraphs) ? graphRecord.subgraphs : undefined,
        }
    }
    return {
        status,
        graph,
        diagnostics: parseDiagnosticList(record.diagnostics, endpoint, 'diagnostics'),
        errors: parseDiagnosticList(record.errors, endpoint, 'errors'),
        error: asOptionalString(record.error),
    }
}

export function parsePipelineStartResponse(payload: unknown, endpoint = '/pipelines'): PipelineStartResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: asOptionalString(record.pipeline_id),
        run_id: asOptionalString(record.run_id),
        working_directory: asOptionalString(record.working_directory),
        model: asOptionalString(record.model),
        diagnostics: parseDiagnosticList(record.diagnostics, endpoint, 'diagnostics'),
        errors: parseDiagnosticList(record.errors, endpoint, 'errors'),
        error: asOptionalString(record.error),
    }
}

export function parsePipelineStatusResponse(payload: unknown, endpoint = '/pipelines/{id}'): PipelineStatusResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        status: expectString(record.status, endpoint, 'status'),
        flow_name: asOptionalString(record.flow_name),
        working_directory: asOptionalString(record.working_directory),
        model: asOptionalString(record.model),
        last_error: asOptionalNullableString(record.last_error),
        completed_nodes: asOptionalStringArray(record.completed_nodes),
        started_at: asOptionalString(record.started_at),
        ended_at: asOptionalNullableString(record.ended_at),
        result: asOptionalNullableString(record.result),
    }
}

export function parsePipelineCancelResponse(payload: unknown, endpoint = '/pipelines/{id}/cancel'): PipelineCancelResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
    }
}

export function parsePipelineCheckpointResponse(
    payload: unknown,
    endpoint = '/pipelines/{id}/checkpoint',
): PipelineCheckpointResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        checkpoint: expectObjectRecord(record.checkpoint, endpoint),
    }
}

export function parsePipelineContextResponse(payload: unknown, endpoint = '/pipelines/{id}/context'): PipelineContextResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        context: expectObjectRecord(record.context, endpoint),
    }
}

export function parsePipelineQuestionsResponse(
    payload: unknown,
    endpoint = '/pipelines/{id}/questions',
): PipelineQuestionsResponse {
    const record = expectObjectRecord(payload, endpoint)
    const rawQuestions = record.questions
    if (!Array.isArray(rawQuestions)) {
        throw new ApiSchemaError(endpoint, 'Expected "questions" to be an array.')
    }
    return {
        questions: rawQuestions
            .map((question) => asUnknownRecord(question))
            .filter((question): question is Record<string, unknown> => question !== null),
    }
}

export function parsePipelineAnswerResponse(
    payload: unknown,
    endpoint = '/pipelines/{id}/questions/{qid}/answer',
): PipelineAnswerResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        question_id: expectString(record.question_id, endpoint, 'question_id'),
    }
}

export function parsePipelineGraphResponse(payload: unknown, endpoint = '/pipelines/{id}/graph'): PipelineGraphResponse {
    if (typeof payload !== 'string') {
        throw new ApiSchemaError(endpoint, 'Expected SVG/text response body.')
    }
    if (payload.trim().length === 0) {
        throw new ApiSchemaError(endpoint, 'Expected non-empty SVG/text response body.')
    }
    return payload
}

function parseRunRecord(payload: unknown): RunRecordResponse | null {
    const record = asUnknownRecord(payload)
    if (!record) {
        return null
    }
    if (typeof record.run_id !== 'string' || typeof record.status !== 'string') {
        return null
    }
    return {
        run_id: record.run_id,
        flow_name: typeof record.flow_name === 'string' ? record.flow_name : '',
        status: record.status,
        result: asOptionalNullableString(record.result),
        working_directory: typeof record.working_directory === 'string' ? record.working_directory : '',
        project_path: asOptionalString(record.project_path),
        git_branch: asOptionalNullableString(record.git_branch),
        git_commit: asOptionalNullableString(record.git_commit),
        spec_id: asOptionalNullableString(record.spec_id),
        plan_id: asOptionalNullableString(record.plan_id),
        model: typeof record.model === 'string' ? record.model : '',
        started_at: typeof record.started_at === 'string' ? record.started_at : '',
        ended_at: asOptionalNullableString(record.ended_at),
        last_error: asOptionalString(record.last_error),
        token_usage: typeof record.token_usage === 'number'
            ? record.token_usage
            : record.token_usage === null
                ? null
                : undefined,
    }
}

export function parseRunsListResponse(payload: unknown, endpoint = '/runs'): RunsListResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (!Array.isArray(record.runs)) {
        throw new ApiSchemaError(endpoint, 'Expected "runs" to be an array.')
    }
    return {
        runs: record.runs
            .map((run) => parseRunRecord(run))
            .filter((run): run is RunRecordResponse => run !== null),
    }
}

export function parseRuntimeStatusResponse(payload: unknown, endpoint = '/status'): RuntimeStatusResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        last_error: asOptionalNullableString(record.last_error),
        last_working_directory: asOptionalNullableString(record.last_working_directory),
        last_model: asOptionalNullableString(record.last_model),
        last_completed_nodes: asOptionalStringArray(record.last_completed_nodes) ?? null,
        last_flow_name: asOptionalNullableString(record.last_flow_name),
        last_run_id: asOptionalNullableString(record.last_run_id),
    }
}

function parseConversationTurnResponse(value: unknown, endpoint: string): ConversationTurnResponse | null {
    const record = asUnknownRecord(value)
    if (!record) {
        return null
    }
    const role = typeof record.role === 'string' ? record.role : ''
    if (role !== 'user' && role !== 'assistant' && role !== 'system') {
        return null
    }
    const kind = record.kind === 'spec_edit_proposal' || record.kind === 'execution_card' || record.kind === 'message'
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

function parseConversationTurnEventResponse(value: unknown, endpoint: string): ConversationTurnEventResponse | null {
    const record = asUnknownRecord(value)
    if (
        !record
        || typeof record.id !== 'string'
        || typeof record.turn_id !== 'string'
        || typeof record.sequence !== 'number'
        || typeof record.timestamp !== 'string'
        || typeof record.kind !== 'string'
    ) {
        return null
    }
    const kind = record.kind === 'assistant_delta'
        || record.kind === 'assistant_completed'
        || record.kind === 'assistant_failed'
        || record.kind === 'tool_call_started'
        || record.kind === 'tool_call_updated'
        || record.kind === 'tool_call_completed'
        || record.kind === 'tool_call_failed'
        || record.kind === 'retry_started'
        ? record.kind
        : null
    if (!kind) {
        return null
    }
    const toolCall = asUnknownRecord(record.tool_call)
    return {
        id: record.id,
        turn_id: record.turn_id,
        sequence: record.sequence,
        timestamp: record.timestamp,
        kind,
        content_delta: asOptionalNullableString(record.content_delta),
        message: asOptionalNullableString(record.message),
        tool_call_id: asOptionalNullableString(record.tool_call_id),
        tool_call: toolCall && typeof toolCall.title === 'string' && typeof toolCall.id === 'string'
            ? {
                id: toolCall.id,
                kind: toolCall.kind === 'file_change' ? 'file_change' : 'command_execution',
                status: toolCall.status === 'running' || toolCall.status === 'failed' ? toolCall.status : 'completed',
                title: toolCall.title,
                command: asOptionalNullableString(toolCall.command),
                output: asOptionalNullableString(toolCall.output),
                file_paths: Array.isArray(toolCall.file_paths) ? toolCall.file_paths.map((entry) => String(entry)) : [],
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
            .map((entry) => ({
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

function parseConversationSummaryResponse(value: unknown, endpoint: string): ConversationSummaryResponse | null {
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
        project_path: record.project_path,
        title: record.title,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_message_preview: asOptionalNullableString(record.last_message_preview),
    }
}

export function parseConversationSummaryListResponse(
    payload: unknown,
    endpoint = '/api/projects/conversations',
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
    endpoint = '/api/conversations/{id}',
): ConversationDeleteResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status') === 'deleted' ? 'deleted' : 'deleted',
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
    }
}

export function parseProjectDirectoryPickResponse(
    payload: unknown,
    endpoint = '/api/projects/pick-directory',
): ProjectDirectoryPickResponse {
    const record = expectObjectRecord(payload, endpoint)
    const status = expectString(record.status, endpoint, 'status')
    if (status === 'canceled') {
        return { status: 'canceled' }
    }
    if (status === 'selected') {
        return {
            status: 'selected',
            directory_path: expectString(record.directory_path, endpoint, 'directory_path'),
        }
    }
    throw new ApiSchemaError(endpoint, `Expected "status" to be "selected" or "canceled"; got "${status}".`)
}

export function parseConversationSnapshotResponse(
    payload: unknown,
    endpoint = '/api/conversations/{id}',
): ConversationSnapshotResponse {
    const record = expectObjectRecord(payload, endpoint)
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
    const turn_events = Array.isArray(record.turn_events)
        ? record.turn_events
            .map((entry) => parseConversationTurnEventResponse(entry, endpoint))
            .filter((entry): entry is ConversationTurnEventResponse => entry !== null)
        : []
    const spec_edit_proposals = Array.isArray(record.spec_edit_proposals)
        ? record.spec_edit_proposals
            .map((entry) => parseSpecEditProposalResponse(entry))
            .filter((entry): entry is SpecEditProposalResponse => entry !== null)
        : []
    const execution_cards = Array.isArray(record.execution_cards)
        ? record.execution_cards
            .map((entry) => parseExecutionCardResponse(entry))
            .filter((entry): entry is ExecutionCardResponse => entry !== null)
        : []
    const executionWorkflowRecord = asUnknownRecord(record.execution_workflow) || {}
    return {
        conversation_id: expectString(record.conversation_id, endpoint, 'conversation_id'),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
        title: asOptionalString(record.title) || 'New thread',
        created_at: asOptionalString(record.created_at) || '',
        updated_at: asOptionalString(record.updated_at) || asOptionalString(record.created_at) || '',
        turns,
        turn_events,
        event_log,
        spec_edit_proposals,
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
    endpoint = '/api/conversations/{id}/events',
): ConversationTurnUpsertEventResponse | ConversationTurnEventStreamResponse | null {
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
    if (type === 'turn_event') {
        const event = parseConversationTurnEventResponse(record.event, endpoint)
        if (
            !event
            || typeof record.conversation_id !== 'string'
            || typeof record.project_path !== 'string'
            || typeof record.title !== 'string'
            || typeof record.updated_at !== 'string'
        ) {
            return null
        }
        return {
            type: 'turn_event',
            conversation_id: record.conversation_id,
            project_path: record.project_path,
            title: record.title,
            updated_at: record.updated_at,
            event,
        }
    }
    return null
}

export async function fetchFlowListValidated(): Promise<string[]> {
    return fetchJsonWithValidation('/api/flows', undefined, '/api/flows', parseFlowListResponse)
}

export async function fetchFlowPayloadValidated(flowName: string): Promise<FlowPayloadResponse> {
    const url = `/api/flows/${encodeURIComponent(flowName)}`
    return fetchJsonWithValidation(url, undefined, '/api/flows/{name}', parseFlowPayloadResponse)
}

export async function fetchPreviewValidated(flowContent: string): Promise<PreviewResponsePayload> {
    return fetchJsonWithValidation(
        '/preview',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ flow_content: flowContent }),
        },
        '/preview',
        parsePreviewResponse,
    )
}

export async function fetchPipelineStartValidated(payload: Record<string, unknown>): Promise<PipelineStartResponse> {
    return fetchJsonWithValidation(
        '/pipelines',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/pipelines',
        parsePipelineStartResponse,
    )
}

export async function fetchPipelineStatusValidated(pipelineId: string): Promise<PipelineStatusResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}`
    return fetchJsonWithValidation(url, undefined, '/pipelines/{id}', parsePipelineStatusResponse)
}

export async function fetchPipelineCancelValidated(pipelineId: string): Promise<PipelineCancelResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/cancel`
    return fetchJsonWithValidation(
        url,
        { method: 'POST' },
        '/pipelines/{id}/cancel',
        parsePipelineCancelResponse,
    )
}

export async function fetchPipelineCheckpointValidated(pipelineId: string): Promise<PipelineCheckpointResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/checkpoint`
    return fetchJsonWithValidation(url, undefined, '/pipelines/{id}/checkpoint', parsePipelineCheckpointResponse)
}

export async function fetchPipelineContextValidated(pipelineId: string): Promise<PipelineContextResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/context`
    return fetchJsonWithValidation(url, undefined, '/pipelines/{id}/context', parsePipelineContextResponse)
}

export async function fetchPipelineQuestionsValidated(pipelineId: string): Promise<PipelineQuestionsResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/questions`
    return fetchJsonWithValidation(url, undefined, '/pipelines/{id}/questions', parsePipelineQuestionsResponse)
}

export async function fetchPipelineAnswerValidated(
    pipelineId: string,
    questionId: string,
    selectedValue: string,
): Promise<PipelineAnswerResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/questions/${encodeURIComponent(questionId)}/answer`
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                question_id: questionId,
                selected_value: selectedValue,
            }),
        },
        '/pipelines/{id}/questions/{qid}/answer',
        parsePipelineAnswerResponse,
    )
}

export async function fetchPipelineGraphValidated(pipelineId: string): Promise<PipelineGraphResponse> {
    const url = `/pipelines/${encodeURIComponent(pipelineId)}/graph`
    return fetchTextWithValidation(url, undefined, '/pipelines/{id}/graph', parsePipelineGraphResponse)
}

export async function fetchRunsListValidated(): Promise<RunsListResponse> {
    return fetchJsonWithValidation('/runs', undefined, '/runs', parseRunsListResponse)
}

export async function fetchRuntimeStatusValidated(): Promise<RuntimeStatusResponse> {
    return fetchJsonWithValidation('/status', undefined, '/status', parseRuntimeStatusResponse)
}

export async function fetchConversationSnapshotValidated(
    conversationId: string,
    projectPath: string,
): Promise<ConversationSnapshotResponse> {
    const url = `/api/conversations/${encodeURIComponent(conversationId)}?project_path=${encodeURIComponent(projectPath)}`
    return fetchJsonWithValidation(url, undefined, '/api/conversations/{id}', parseConversationSnapshotResponse)
}

export async function deleteConversationValidated(
    conversationId: string,
    projectPath: string,
): Promise<ConversationDeleteResponse> {
    const url = `/api/conversations/${encodeURIComponent(conversationId)}?project_path=${encodeURIComponent(projectPath)}`
    return fetchJsonWithValidation(
        url,
        {
            method: 'DELETE',
        },
        '/api/conversations/{id}',
        parseConversationDeleteResponse,
    )
}

export async function fetchProjectConversationListValidated(
    projectPath: string,
): Promise<ConversationSummaryResponse[]> {
    const url = `/api/projects/conversations?project_path=${encodeURIComponent(projectPath)}`
    return fetchJsonWithValidation(url, undefined, '/api/projects/conversations', parseConversationSummaryListResponse)
}

export async function pickProjectDirectoryValidated(): Promise<ProjectDirectoryPickResponse> {
    return fetchJsonWithValidation(
        '/api/projects/pick-directory',
        {
            method: 'POST',
        },
        '/api/projects/pick-directory',
        parseProjectDirectoryPickResponse,
    )
}

export async function sendConversationTurnValidated(
    conversationId: string,
    payload: { project_path: string; message: string; model?: string | null },
): Promise<ConversationSnapshotResponse> {
    const url = `/api/conversations/${encodeURIComponent(conversationId)}/turns`
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/api/conversations/{id}/turns',
        parseConversationSnapshotResponse,
    )
}

export async function approveSpecEditProposalValidated(
    conversationId: string,
    proposalId: string,
    payload: { project_path: string; model?: string | null; flow_source?: string | null },
): Promise<ConversationSnapshotResponse> {
    const url = `/api/conversations/${encodeURIComponent(conversationId)}/spec-edit-proposals/${encodeURIComponent(proposalId)}/approve`
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/api/conversations/{id}/spec-edit-proposals/{proposalId}/approve',
        parseConversationSnapshotResponse,
    )
}

export async function rejectSpecEditProposalValidated(
    conversationId: string,
    proposalId: string,
    payload: { project_path: string },
): Promise<ConversationSnapshotResponse> {
    const url = `/api/conversations/${encodeURIComponent(conversationId)}/spec-edit-proposals/${encodeURIComponent(proposalId)}/reject`
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/api/conversations/{id}/spec-edit-proposals/{proposalId}/reject',
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
    const url = `/api/conversations/${encodeURIComponent(conversationId)}/execution-cards/${encodeURIComponent(executionCardId)}/review`
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/api/conversations/{id}/execution-cards/{executionCardId}/review',
        parseConversationSnapshotResponse,
    )
}
