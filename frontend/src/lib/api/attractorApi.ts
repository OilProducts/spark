import type { CanonicalPreviewGraphPayload } from '@/lib/canonicalFlowModel'
import { parseConversationSegmentResponse } from './conversationsApi'
import type { ConversationSegmentResponse } from './conversationsApi'
import { encodeFlowPath } from '@/lib/flowPaths'
import type { PipelineContinuePayload, PipelineStartPayload } from '@/lib/pipelineStartPayload'
import {
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    asOptionalStringArray,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
    fetchJsonWithValidation,
    fetchTextWithValidation,
    parseDiagnosticList,
} from './shared'

export const ATTRACTOR_BASE = '/attractor'
export const ATTRACTOR_API_BASE = `${ATTRACTOR_BASE}/api`

function attractorUrl(path: string): string {
    return `${ATTRACTOR_BASE}${path}`
}

export function pipelineJournalUrl(
    pipelineId: string,
    options?: {
        limit?: number
        beforeSequence?: number | null
    },
): string {
    const params = new URLSearchParams()
    if (typeof options?.limit === 'number' && Number.isFinite(options.limit)) {
        params.set('limit', String(options.limit))
    }
    if (typeof options?.beforeSequence === 'number' && Number.isFinite(options.beforeSequence)) {
        params.set('before_sequence', String(options.beforeSequence))
    }
    const query = params.toString()
    return `${attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/journal`)}${query ? `?${query}` : ''}`
}

export function pipelineTranscriptUrl(pipelineId: string): string {
    return attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/transcript`)
}

export interface FlowPayloadResponse {
    name: string
    content: string
}

export interface SaveFlowApiResponse {
    ok: boolean
    statusCode: number
    payload: unknown
}

export interface PreviewResponsePayload {
    status: string
    flow?: Record<string, unknown>
    graph?: CanonicalPreviewGraphPayload
    diagnostics?: import('@/state/store-types').DiagnosticEntry[]
    errors?: import('@/state/store-types').DiagnosticEntry[]
    error?: string
}

export interface PipelineStartResponse {
    status: string
    pipeline_id?: string
    run_id?: string
    working_directory?: string
    model?: string
    provider?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    execution_mode?: string
    execution_profile_id?: string
    execution_container_image?: string | null
    execution_lock?: {
        scope: string
        key: string
        conflict_policy: string
        identity: string
        state: string
        queue_position?: number | null
    } | null
    diagnostics?: import('@/state/store-types').DiagnosticEntry[]
    errors?: import('@/state/store-types').DiagnosticEntry[]
    error?: string
}

export interface TokenUsageBucketResponse {
    input_tokens: number
    cached_input_tokens: number
    output_tokens: number
    total_tokens: number
}

export interface TokenUsageBreakdownResponse extends TokenUsageBucketResponse {
    by_model: Record<string, TokenUsageBucketResponse>
}

export interface ModelEstimatedCostResponse {
    currency: string
    amount: number | null
    status: 'estimated' | 'unpriced'
}

export interface EstimatedModelCostResponse {
    currency: string
    amount: number
    status: 'estimated' | 'partial_unpriced' | 'unpriced'
    unpriced_models: string[]
    by_model?: Record<string, ModelEstimatedCostResponse>
}

export interface PipelineStatusResponse {
    pipeline_id: string
    run_id: string
    status: string
    outcome?: 'success' | 'failure' | null
    outcome_reason_code?: string | null
    outcome_reason_message?: string | null
    flow_name?: string
    working_directory?: string
    project_path?: string
    git_branch?: string | null
    git_commit?: string | null
    spec_id?: string | null
    plan_id?: string | null
    model?: string
    provider?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    last_error?: string | null
    token_usage?: number | null
    token_usage_breakdown?: TokenUsageBreakdownResponse | null
    estimated_model_cost?: EstimatedModelCostResponse | null
    current_node?: string | null
    completed_nodes?: string[]
    progress?: {
        current_node?: string | null
        completed_nodes?: string[]
        completed_count?: number
    }
    started_at?: string
    ended_at?: string | null
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

export interface PipelineResultResponse {
    run_id: string
    status: string
    state: 'pending' | 'ready' | 'unavailable' | 'error'
    source_node_id?: string | null
    source_artifact_path?: string | null
    display_mode?: 'raw' | 'summary' | null
    body_markdown: string
    summary_enabled: boolean
    summary_prompt?: string | null
    summary_error?: string | null
    error?: string | null
}

export interface PipelineQuestionsResponse {
    questions: Array<Record<string, unknown>>
}

export interface RunJournalEntryResponse {
    id: string
    sequence: number
    emitted_at: string
    kind: string
    raw_type: string
    severity: string
    summary: string
    node_id?: string | null
    stage_index?: number | null
    source_scope?: string | null
    source_parent_node_id?: string | null
    source_flow_name?: string | null
    question_id?: string | null
    payload: Record<string, unknown>
}

export interface RunJournalPageResponse {
    pipeline_id: string
    entries: RunJournalEntryResponse[]
    oldest_sequence?: number | null
    newest_sequence?: number | null
    has_older: boolean
}

export interface RunTranscriptResponse {
    pipeline_id: string
    entries: Record<string, unknown>[]
}

export interface PipelineAnswerResponse {
    status: string
    pipeline_id: string
    question_id: string
}

export type PipelineGraphResponse = string
export type PipelineGraphPreviewResponse = PreviewResponsePayload

export interface RunRecordResponse {
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
    llm_profile?: string | null
    reasoning_effort?: string | null
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
    token_usage_breakdown?: TokenUsageBreakdownResponse | null
    estimated_model_cost?: EstimatedModelCostResponse | null
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
}

export interface PreviewRequestOptions {
    flowName?: string | null
    expandChildren?: boolean
}

export interface RunsListResponse {
    runs: RunRecordResponse[]
}

export interface RuntimeStatusResponse {
    status: string
    outcome?: 'success' | 'failure' | null
    outcome_reason_code?: string | null
    outcome_reason_message?: string | null
    last_error?: string | null
    last_working_directory?: string | null
    last_model?: string | null
    last_completed_nodes?: string[] | null
    last_flow_name?: string | null
}

function parseRunJournalEntryResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/journal',
): RunJournalEntryResponse | null {
    const record = asUnknownRecord(payload)
    if (!record) {
        return null
    }
    const sequence = record.sequence
    if (
        typeof record.id !== 'string'
        || typeof sequence !== 'number'
        || !Number.isFinite(sequence)
        || typeof record.emitted_at !== 'string'
        || typeof record.kind !== 'string'
        || typeof record.raw_type !== 'string'
        || typeof record.severity !== 'string'
        || typeof record.summary !== 'string'
    ) {
        return null
    }
    return {
        id: record.id,
        sequence,
        emitted_at: record.emitted_at,
        kind: record.kind,
        raw_type: record.raw_type,
        severity: record.severity,
        summary: record.summary,
        node_id: asOptionalNullableString(record.node_id),
        stage_index: typeof record.stage_index === 'number'
            ? record.stage_index
            : record.stage_index === null
                ? null
                : undefined,
        source_scope: asOptionalNullableString(record.source_scope),
        source_parent_node_id: asOptionalNullableString(record.source_parent_node_id),
        source_flow_name: asOptionalNullableString(record.source_flow_name),
        question_id: asOptionalNullableString(record.question_id),
        payload: expectObjectRecord(record.payload, endpoint),
    }
}

export function parseFlowListResponse(payload: unknown, endpoint = '/attractor/api/flows'): string[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of flow names.')
    }
    if (!payload.every((entry) => typeof entry === 'string')) {
        throw new ApiSchemaError(endpoint, 'Expected every flow name to be a string.')
    }
    return [...payload]
}

export function parseFlowPayloadResponse(payload: unknown, endpoint = '/attractor/api/flows/{name}'): FlowPayloadResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        name: typeof record.name === 'string' ? record.name : '',
        content: expectString(record.content, endpoint, 'content'),
    }
}

export function parsePreviewResponse(payload: unknown, endpoint = '/attractor/preview'): PreviewResponsePayload {
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
            metadata: asUnknownRecord(graphRecord.metadata),
            child_previews: asUnknownRecord(graphRecord.child_previews),
        }
    }
    return {
        status,
        flow: asUnknownRecord(record.flow) ?? undefined,
        graph,
        diagnostics: parseDiagnosticList(record.diagnostics, endpoint, 'diagnostics'),
        errors: parseDiagnosticList(record.errors, endpoint, 'errors'),
        error: asOptionalString(record.error),
    }
}

export function parsePipelineStartResponse(payload: unknown, endpoint = '/attractor/pipelines'): PipelineStartResponse {
    const record = expectObjectRecord(payload, endpoint)
    const executionLockRecord = asUnknownRecord(record.execution_lock)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: asOptionalString(record.pipeline_id),
        run_id: asOptionalString(record.run_id),
        working_directory: asOptionalString(record.working_directory),
        model: asOptionalString(record.model),
        provider: asOptionalNullableString(record.provider),
        llm_provider: asOptionalNullableString(record.llm_provider),
        llm_profile: asOptionalNullableString(record.llm_profile),
        reasoning_effort: asOptionalNullableString(record.reasoning_effort),
        execution_mode: asOptionalString(record.execution_mode),
        execution_profile_id: asOptionalString(record.execution_profile_id),
        execution_container_image: asOptionalNullableString(record.execution_container_image),
        execution_lock: executionLockRecord
            ? {
                scope: expectString(executionLockRecord.scope, endpoint, 'execution_lock.scope'),
                key: expectString(executionLockRecord.key, endpoint, 'execution_lock.key'),
                conflict_policy: expectString(executionLockRecord.conflict_policy, endpoint, 'execution_lock.conflict_policy'),
                identity: expectString(executionLockRecord.identity, endpoint, 'execution_lock.identity'),
                state: expectString(executionLockRecord.state, endpoint, 'execution_lock.state'),
                queue_position: typeof executionLockRecord.queue_position === 'number'
                    ? executionLockRecord.queue_position
                    : executionLockRecord.queue_position === null
                        ? null
                        : undefined,
            }
            : null,
        diagnostics: parseDiagnosticList(record.diagnostics, endpoint, 'diagnostics'),
        errors: parseDiagnosticList(record.errors, endpoint, 'errors'),
        error: asOptionalString(record.error),
    }
}

export function parsePipelineStatusResponse(payload: unknown, endpoint = '/attractor/pipelines/{id}'): PipelineStatusResponse {
    const record = expectObjectRecord(payload, endpoint)
    const pipelineId = expectString(record.pipeline_id, endpoint, 'pipeline_id')
    const progressRecord = asUnknownRecord(record.progress)
    const progressCurrentNode = asOptionalNullableString(progressRecord?.current_node) ?? null
    const resolvedCurrentNode = progressCurrentNode ?? asOptionalNullableString(record.current_node) ?? null
    const progressCompletedNodes = asOptionalStringArray(progressRecord?.completed_nodes)
    const rawProgressCompletedCount = progressRecord?.completed_count
    const progressCompletedCount = typeof rawProgressCompletedCount === 'number' && Number.isFinite(rawProgressCompletedCount)
        ? rawProgressCompletedCount
        : undefined
    const runRecord = parseRunRecord({
        ...record,
        run_id: typeof record.run_id === 'string' ? record.run_id : pipelineId,
    }, { currentNode: resolvedCurrentNode })
    if (!runRecord) {
        throw new ApiSchemaError(endpoint, 'Expected a valid run detail payload.')
    }
    const completedNodes = asOptionalStringArray(record.completed_nodes) ?? progressCompletedNodes
    return {
        pipeline_id: pipelineId,
        ...runRecord,
        current_node: resolvedCurrentNode,
        completed_nodes: completedNodes,
        progress: progressRecord
            ? {
                current_node: progressCurrentNode,
                completed_nodes: progressCompletedNodes ?? [],
                completed_count: progressCompletedCount,
            }
            : undefined,
    }
}

export function parsePipelineCancelResponse(payload: unknown, endpoint = '/attractor/pipelines/{id}/cancel'): PipelineCancelResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
    }
}

export function parsePipelineCheckpointResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/checkpoint',
): PipelineCheckpointResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        checkpoint: expectObjectRecord(record.checkpoint, endpoint),
    }
}

export function parsePipelineContextResponse(payload: unknown, endpoint = '/attractor/pipelines/{id}/context'): PipelineContextResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        context: expectObjectRecord(record.context, endpoint),
    }
}

export function parsePipelineResultResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/result',
): PipelineResultResponse {
    const record = expectObjectRecord(payload, endpoint)
    const rawState = expectString(record.state, endpoint, 'state')
    const state: PipelineResultResponse['state'] =
        rawState === 'pending' || rawState === 'ready' || rawState === 'unavailable' || rawState === 'error'
            ? rawState
            : 'error'
    const rawDisplayMode = asOptionalNullableString(record.display_mode)
    const displayMode: PipelineResultResponse['display_mode'] =
        rawDisplayMode === 'raw' || rawDisplayMode === 'summary' ? rawDisplayMode : null
    return {
        run_id: expectString(record.run_id, endpoint, 'run_id'),
        status: expectString(record.status, endpoint, 'status'),
        state,
        source_node_id: asOptionalNullableString(record.source_node_id),
        source_artifact_path: asOptionalNullableString(record.source_artifact_path),
        display_mode: displayMode,
        body_markdown: typeof record.body_markdown === 'string' ? record.body_markdown : '',
        summary_enabled: record.summary_enabled === true,
        summary_prompt: asOptionalNullableString(record.summary_prompt),
        summary_error: asOptionalNullableString(record.summary_error),
        error: asOptionalNullableString(record.error),
    }
}

export function parsePipelineQuestionsResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/questions',
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

export function parseRunJournalPageResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/journal',
): RunJournalPageResponse {
    const record = expectObjectRecord(payload, endpoint)
    const rawEntries = record.entries
    if (!Array.isArray(rawEntries)) {
        throw new ApiSchemaError(endpoint, 'Expected "entries" to be an array.')
    }
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        entries: rawEntries
            .map((entry) => parseRunJournalEntryResponse(entry, endpoint))
            .filter((entry): entry is RunJournalEntryResponse => entry !== null),
        oldest_sequence: typeof record.oldest_sequence === 'number'
            ? record.oldest_sequence
            : record.oldest_sequence === null
                ? null
                : undefined,
        newest_sequence: typeof record.newest_sequence === 'number'
            ? record.newest_sequence
            : record.newest_sequence === null
                ? null
                : undefined,
        has_older: record.has_older === true,
    }
}

export function parseRunTranscriptResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/transcript',
): RunTranscriptResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (!Array.isArray(record.entries)) {
        throw new ApiSchemaError(endpoint, 'Expected "entries" to be an array.')
    }
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        entries: record.entries
            .map((entry) => asUnknownRecord(entry))
            .filter((entry): entry is Record<string, unknown> => entry !== null),
    }
}

export function parsePipelineAnswerResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/questions/{qid}/answer',
): PipelineAnswerResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        question_id: expectString(record.question_id, endpoint, 'question_id'),
    }
}

export function parsePipelineGraphResponse(payload: unknown, endpoint = '/attractor/pipelines/{id}/graph'): PipelineGraphResponse {
    if (typeof payload !== 'string') {
        throw new ApiSchemaError(endpoint, 'Expected SVG/text response body.')
    }
    if (payload.trim().length === 0) {
        throw new ApiSchemaError(endpoint, 'Expected non-empty SVG/text response body.')
    }
    return payload
}

export function parsePipelineGraphPreviewResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/graph-preview',
): PipelineGraphPreviewResponse {
    return parsePreviewResponse(payload, endpoint)
}

function parseTokenUsageBucket(value: unknown): TokenUsageBucketResponse | undefined {
    const record = asUnknownRecord(value)
    if (!record) {
        return undefined
    }
    const inputTokens = typeof record.input_tokens === 'number'
        ? record.input_tokens
        : typeof record.inputTokens === 'number'
            ? record.inputTokens
            : 0
    const cachedInputTokens = typeof record.cached_input_tokens === 'number'
        ? record.cached_input_tokens
        : typeof record.cachedInputTokens === 'number'
            ? record.cachedInputTokens
            : 0
    const outputTokens = typeof record.output_tokens === 'number'
        ? record.output_tokens
        : typeof record.outputTokens === 'number'
            ? record.outputTokens
            : 0
    const totalTokens = typeof record.total_tokens === 'number'
        ? record.total_tokens
        : typeof record.totalTokens === 'number'
            ? record.totalTokens
            : inputTokens + outputTokens
    return {
        input_tokens: inputTokens,
        cached_input_tokens: cachedInputTokens,
        output_tokens: outputTokens,
        total_tokens: totalTokens,
    }
}

function parseTokenUsageBreakdown(value: unknown): TokenUsageBreakdownResponse | null | undefined {
    if (value === null) {
        return null
    }
    const aggregate = parseTokenUsageBucket(value)
    const record = asUnknownRecord(value)
    if (!aggregate || !record) {
        return undefined
    }
    const byModelRecord = asUnknownRecord(record.by_model)
    const byModel: Record<string, TokenUsageBucketResponse> = {}
    if (byModelRecord) {
        Object.entries(byModelRecord).forEach(([modelId, usageValue]) => {
            const parsedUsage = parseTokenUsageBucket(usageValue)
            if (parsedUsage) {
                byModel[modelId] = parsedUsage
            }
        })
    }
    return {
        ...aggregate,
        by_model: byModel,
    }
}

function parseEstimatedModelCost(value: unknown): EstimatedModelCostResponse | null | undefined {
    if (value === null) {
        return null
    }
    const record = asUnknownRecord(value)
    if (!record) {
        return undefined
    }
    const rawStatus = typeof record.status === 'string' ? record.status : 'unpriced'
    const status: EstimatedModelCostResponse['status'] =
        rawStatus === 'estimated' || rawStatus === 'partial_unpriced' || rawStatus === 'unpriced'
            ? rawStatus
            : 'unpriced'
    const byModelRecord = asUnknownRecord(record.by_model)
    const byModel: Record<string, ModelEstimatedCostResponse> = {}
    if (byModelRecord) {
        Object.entries(byModelRecord).forEach(([modelId, modelValue]) => {
            const parsedModel = asUnknownRecord(modelValue)
            if (!parsedModel) {
                return
            }
            const modelStatus = parsedModel.status === 'estimated' ? 'estimated' : 'unpriced'
            const rawAmount = parsedModel.amount
            byModel[modelId] = {
                currency: typeof parsedModel.currency === 'string' ? parsedModel.currency : 'USD',
                amount: typeof rawAmount === 'number' ? rawAmount : rawAmount === null ? null : null,
                status: modelStatus,
            }
        })
    }
    return {
        currency: typeof record.currency === 'string' ? record.currency : 'USD',
        amount: typeof record.amount === 'number' ? record.amount : 0,
        status,
        unpriced_models: Array.isArray(record.unpriced_models)
            ? record.unpriced_models.filter((modelId): modelId is string => typeof modelId === 'string')
            : [],
        by_model: Object.keys(byModel).length > 0 ? byModel : undefined,
    }
}

function parseRunRecord(
    payload: unknown,
    options: { currentNode?: string | null } = {},
): RunRecordResponse | null {
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
        outcome: asOptionalNullableString(record.outcome) as RunRecordResponse['outcome'],
        outcome_reason_code: asOptionalNullableString(record.outcome_reason_code),
        outcome_reason_message: asOptionalNullableString(record.outcome_reason_message),
        working_directory: typeof record.working_directory === 'string' ? record.working_directory : '',
        project_path: asOptionalString(record.project_path),
        git_branch: asOptionalNullableString(record.git_branch),
        git_commit: asOptionalNullableString(record.git_commit),
        spec_id: asOptionalNullableString(record.spec_id),
        plan_id: asOptionalNullableString(record.plan_id),
        model: typeof record.model === 'string' ? record.model : '',
        provider: asOptionalNullableString(record.provider),
        llm_provider: asOptionalNullableString(record.llm_provider),
        llm_profile: asOptionalNullableString(record.llm_profile),
        reasoning_effort: asOptionalNullableString(record.reasoning_effort),
        started_at: typeof record.started_at === 'string' ? record.started_at : '',
        ended_at: asOptionalNullableString(record.ended_at),
        last_error: asOptionalString(record.last_error),
        token_usage: typeof record.token_usage === 'number'
            ? record.token_usage
            : record.token_usage === null
                ? null
                : undefined,
        token_usage_breakdown: parseTokenUsageBreakdown(record.token_usage_breakdown),
        estimated_model_cost: parseEstimatedModelCost(record.estimated_model_cost),
        current_node: options.currentNode,
        continued_from_run_id: asOptionalNullableString(record.continued_from_run_id),
        continued_from_node: asOptionalNullableString(record.continued_from_node),
        continued_from_flow_mode: asOptionalNullableString(record.continued_from_flow_mode),
        continued_from_flow_name: asOptionalNullableString(record.continued_from_flow_name),
        parent_run_id: asOptionalNullableString(record.parent_run_id),
        parent_node_id: asOptionalNullableString(record.parent_node_id),
        root_run_id: asOptionalNullableString(record.root_run_id),
        child_invocation_index: typeof record.child_invocation_index === 'number'
            ? record.child_invocation_index
            : record.child_invocation_index === null
                ? null
                : undefined,
        execution_mode: asOptionalString(record.execution_mode),
        execution_profile_id: asOptionalString(record.execution_profile_id),
        execution_container_image: asOptionalNullableString(record.execution_container_image),
        execution_profile_capabilities: record.execution_profile_capabilities,
        execution_lock: asUnknownRecord(record.execution_lock)
            ? {
                scope: asOptionalString(asUnknownRecord(record.execution_lock)?.scope) || '',
                key: asOptionalString(asUnknownRecord(record.execution_lock)?.key) || '',
                conflict_policy: asOptionalString(asUnknownRecord(record.execution_lock)?.conflict_policy) || '',
                identity: asOptionalString(asUnknownRecord(record.execution_lock)?.identity) || '',
                state: asOptionalString(asUnknownRecord(record.execution_lock)?.state) || '',
                queue_position: typeof asUnknownRecord(record.execution_lock)?.queue_position === 'number'
                    ? asUnknownRecord(record.execution_lock)?.queue_position as number
                    : asUnknownRecord(record.execution_lock)?.queue_position === null
                        ? null
                        : undefined,
            }
            : null,
        cleanup_error: asOptionalString(record.cleanup_error),
    }
}

export function parseRunRecordPayload(payload: unknown): RunRecordResponse | null {
    return parseRunRecord(payload)
}

export function parseRunsListResponse(payload: unknown, endpoint = '/attractor/runs'): RunsListResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (!Array.isArray(record.runs)) {
        throw new ApiSchemaError(endpoint, 'Expected "runs" to be an array.')
    }
    return {
        runs: record.runs
            .map((run) => parseRunRecordPayload(run))
            .filter((run): run is RunRecordResponse => run !== null),
    }
}

export function parseRuntimeStatusResponse(payload: unknown, endpoint = '/attractor/status'): RuntimeStatusResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status'),
        outcome: asOptionalNullableString(record.outcome) as RuntimeStatusResponse['outcome'],
        outcome_reason_code: asOptionalNullableString(record.outcome_reason_code),
        outcome_reason_message: asOptionalNullableString(record.outcome_reason_message),
        last_error: asOptionalNullableString(record.last_error),
        last_working_directory: asOptionalNullableString(record.last_working_directory),
        last_model: asOptionalNullableString(record.last_model),
        last_completed_nodes: asOptionalStringArray(record.last_completed_nodes) ?? null,
        last_flow_name: asOptionalNullableString(record.last_flow_name),
    }
}

export async function fetchFlowListValidated(): Promise<string[]> {
    return fetchJsonWithValidation(attractorUrl('/api/flows'), undefined, '/attractor/api/flows', parseFlowListResponse)
}

export async function fetchFlowPayloadValidated(flowName: string, init?: RequestInit): Promise<FlowPayloadResponse> {
    const url = attractorUrl(`/api/flows/${encodeFlowPath(flowName)}`)
    return fetchJsonWithValidation(url, init, '/attractor/api/flows/{name}', parseFlowPayloadResponse)
}

export async function saveFlowValidated(
    name: string,
    content: string,
): Promise<SaveFlowApiResponse> {
    const response = await fetch(attractorUrl('/api/flows'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            name,
            content,
        }),
    })
    let payload: unknown = null
    try {
        payload = await response.json()
    } catch {
        payload = null
    }
    return {
        ok: response.ok,
        statusCode: response.status,
        payload,
    }
}

export async function deleteFlowValidated(flowName: string): Promise<void> {
    const url = attractorUrl(`/api/flows/${encodeFlowPath(flowName)}`)
    await fetchJsonWithValidation(url, { method: 'DELETE' }, '/attractor/api/flows/{name}', () => undefined)
}

export async function fetchPreviewValidated(
    flowContent: string,
    init?: RequestInit,
    options?: PreviewRequestOptions,
): Promise<PreviewResponsePayload> {
    const headers = init?.headers
        ? {
            ...Object.fromEntries(new Headers(init.headers).entries()),
            'Content-Type': 'application/json',
        }
        : { 'Content-Type': 'application/json' }
    const payload: Record<string, unknown> = {
        flow_content: flowContent,
    }
    if (typeof options?.flowName === 'string' && options.flowName.trim()) {
        payload.flow_name = options.flowName
    }
    if (options?.expandChildren) {
        payload.expand_children = true
    }
    return fetchJsonWithValidation(
        attractorUrl('/preview'),
        {
            ...init,
            method: 'POST',
            headers,
            body: JSON.stringify(payload),
        },
        '/attractor/preview',
        parsePreviewResponse,
    )
}

export async function fetchPipelineStartValidated(payload: PipelineStartPayload): Promise<PipelineStartResponse> {
    return fetchJsonWithValidation(
        attractorUrl('/pipelines'),
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/attractor/pipelines',
        parsePipelineStartResponse,
    )
}

export async function fetchPipelineContinueValidated(
    pipelineId: string,
    payload: PipelineContinuePayload,
): Promise<PipelineStartResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/continue`)
    return fetchJsonWithValidation(
        url,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/attractor/pipelines/{id}/continue',
        parsePipelineStartResponse,
    )
}

export async function fetchPipelineRetryValidated(pipelineId: string): Promise<PipelineStartResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/retry`)
    return fetchJsonWithValidation(
        url,
        { method: 'POST' },
        '/attractor/pipelines/{id}/retry',
        parsePipelineStartResponse,
    )
}

export async function fetchPipelineStatusValidated(pipelineId: string, init?: RequestInit): Promise<PipelineStatusResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}`)
    return fetchJsonWithValidation(url, init, '/attractor/pipelines/{id}', parsePipelineStatusResponse)
}

export async function fetchPipelineCancelValidated(pipelineId: string): Promise<PipelineCancelResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/cancel`)
    return fetchJsonWithValidation(
        url,
        { method: 'POST' },
        '/attractor/pipelines/{id}/cancel',
        parsePipelineCancelResponse,
    )
}

export async function fetchPipelineCheckpointValidated(pipelineId: string): Promise<PipelineCheckpointResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/checkpoint`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/checkpoint', parsePipelineCheckpointResponse)
}

export async function fetchPipelineContextValidated(pipelineId: string): Promise<PipelineContextResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/context`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/context', parsePipelineContextResponse)
}

export async function fetchPipelineResultValidated(pipelineId: string): Promise<PipelineResultResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/result`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/result', parsePipelineResultResponse)
}

export async function fetchPipelineQuestionsValidated(pipelineId: string): Promise<PipelineQuestionsResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/questions`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/questions', parsePipelineQuestionsResponse)
}

export async function fetchPipelineJournalValidated(
    pipelineId: string,
    options?: {
        limit?: number
        beforeSequence?: number | null
    },
): Promise<RunJournalPageResponse> {
    return fetchJsonWithValidation(
        pipelineJournalUrl(pipelineId, options),
        undefined,
        '/attractor/pipelines/{id}/journal',
        parseRunJournalPageResponse,
    )
}

export async function fetchPipelineTranscriptValidated(
    pipelineId: string,
): Promise<RunTranscriptResponse> {
    return fetchJsonWithValidation(
        pipelineTranscriptUrl(pipelineId),
        undefined,
        '/attractor/pipelines/{id}/transcript',
        parseRunTranscriptResponse,
    )
}

export async function fetchPipelineAnswerValidated(
    pipelineId: string,
    questionId: string,
    selectedValue: string,
): Promise<PipelineAnswerResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/questions/${encodeURIComponent(questionId)}/answer`)
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
        '/attractor/pipelines/{id}/questions/{qid}/answer',
        parsePipelineAnswerResponse,
    )
}

export async function fetchPipelineGraphValidated(pipelineId: string): Promise<PipelineGraphResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/graph`)
    return fetchTextWithValidation(url, undefined, '/attractor/pipelines/{id}/graph', parsePipelineGraphResponse)
}

export async function fetchPipelineGraphPreviewValidated(
    pipelineId: string,
    init?: RequestInit,
    options?: PreviewRequestOptions,
): Promise<PipelineGraphPreviewResponse> {
    const params = new URLSearchParams()
    if (options?.expandChildren) {
        params.set('expand_children', 'true')
    }
    const query = params.toString()
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/graph-preview${query ? `?${query}` : ''}`)
    return fetchJsonWithValidation(
        url,
        init,
        '/attractor/pipelines/{id}/graph-preview',
        parsePipelineGraphPreviewResponse,
    )
}

export async function fetchRunsListValidated(projectPath?: string | null): Promise<RunsListResponse> {
    const url = projectPath
        ? `${attractorUrl('/runs')}?project_path=${encodeURIComponent(projectPath)}`
        : attractorUrl('/runs')
    return fetchJsonWithValidation(url, undefined, '/attractor/runs', parseRunsListResponse)
}

export async function fetchRuntimeStatusValidated(): Promise<RuntimeStatusResponse> {
    return fetchJsonWithValidation(attractorUrl('/status'), undefined, '/attractor/status', parseRuntimeStatusResponse)
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

export function parseArtifactListResponse(
    payload: unknown,
    endpoint = '/attractor/pipelines/{id}/artifacts',
): ArtifactListResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        pipeline_id: expectString(record.pipeline_id, endpoint, 'pipeline_id'),
        artifacts: Array.isArray(record.artifacts)
            ? record.artifacts
                .map((entry) => expectObjectRecord(entry, endpoint))
                .map((entry) => ({
                    path: expectString(entry.path, endpoint, 'path'),
                    size_bytes: typeof entry.size_bytes === 'number' ? entry.size_bytes : 0,
                    media_type: expectString(entry.media_type, endpoint, 'media_type'),
                    viewable: entry.viewable === true,
                }))
            : [],
    }
}

export async function fetchPipelineArtifactsValidated(pipelineId: string): Promise<ArtifactListResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/artifacts`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/artifacts', parseArtifactListResponse)
}

function encodeArtifactPathSegments(artifactPath: string): string {
    return artifactPath
        .replace(/\\/g, '/')
        .split('/')
        .map((segment) => encodeURIComponent(segment))
        .join('/')
}

export async function fetchPipelineArtifactPreviewValidated(pipelineId: string, artifactPath: string): Promise<string> {
    const encodedPath = encodeArtifactPathSegments(artifactPath)
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/artifacts/${encodedPath}`)
    return fetchTextWithValidation(url, undefined, '/pipelines/{id}/artifacts/{artifact_path}', parsePipelineGraphResponse)
}

export function pipelineArtifactHref(pipelineId: string, artifactPath: string, download = false): string {
    const encodedPath = encodeArtifactPathSegments(artifactPath)
    const query = download ? '?download=1' : ''
    return attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/artifacts/${encodedPath}${query}`)
}

export interface RunTranscriptSegment extends ConversationSegmentResponse {
    node_id: string | null
    attempt: number
    latest_sequence: number
    source_scope: 'root' | 'child'
    source_flow_name: string | null
    source_parent_node_id: string | null
    source_run_id: string | null
}

export interface RunSegmentsResponse {
    run_id: string
    segments: RunTranscriptSegment[]
    newest_sequence: number
}

export function parseRunTranscriptSegment(value: unknown): RunTranscriptSegment | null {
    const base = parseConversationSegmentResponse(value)
    const record = asUnknownRecord(value)
    if (!base || !record) {
        return null
    }
    return {
        ...base,
        node_id: asOptionalNullableString(record.node_id) ?? null,
        attempt: typeof record.attempt === 'number' ? record.attempt : 0,
        latest_sequence: typeof record.latest_sequence === 'number' ? record.latest_sequence : 0,
        source_scope: record.source_scope === 'child' ? 'child' : 'root',
        source_flow_name: asOptionalNullableString(record.source_flow_name) ?? null,
        source_parent_node_id: asOptionalNullableString(record.source_parent_node_id) ?? null,
        source_run_id: asOptionalNullableString(record.source_run_id) ?? null,
    }
}

function parseRunSegmentsResponse(payload: unknown): RunSegmentsResponse {
    const record = expectObjectRecord(payload, '/attractor/pipelines/{id}/segments')
    return {
        run_id: expectString(record.run_id, 'run_id', '/attractor/pipelines/{id}/segments'),
        segments: Array.isArray(record.segments)
            ? record.segments
                .map((entry) => parseRunTranscriptSegment(entry))
                .filter((entry): entry is RunTranscriptSegment => entry !== null)
            : [],
        newest_sequence: typeof record.newest_sequence === 'number' ? record.newest_sequence : 0,
    }
}

export function fetchRunSegmentsValidated(runId: string): Promise<RunSegmentsResponse> {
    return fetchJsonWithValidation(
        attractorUrl(`/pipelines/${encodeURIComponent(runId)}/segments`),
        undefined,
        '/attractor/pipelines/{id}/segments',
        parseRunSegmentsResponse,
    )
}
