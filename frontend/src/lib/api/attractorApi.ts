import type { CanonicalPreviewGraphPayload } from '@/lib/canonicalFlowModel'
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

export function pipelineEventsUrl(pipelineId: string): string {
    return attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/events`)
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
    diagnostics?: import('@/state/store-types').DiagnosticEntry[]
    errors?: import('@/state/store-types').DiagnosticEntry[]
    error?: string
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
    last_error?: string | null
    token_usage?: number | null
    completed_nodes?: string[]
    started_at?: string
    ended_at?: string | null
    continued_from_run_id?: string | null
    continued_from_node?: string | null
    continued_from_flow_mode?: string | null
    continued_from_flow_name?: string | null
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
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
    continued_from_run_id?: string | null
    continued_from_node?: string | null
    continued_from_flow_mode?: string | null
    continued_from_flow_name?: string | null
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
    last_run_id?: string | null
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

export function parsePipelineStartResponse(payload: unknown, endpoint = '/attractor/pipelines'): PipelineStartResponse {
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

export function parsePipelineStatusResponse(payload: unknown, endpoint = '/attractor/pipelines/{id}'): PipelineStatusResponse {
    const record = expectObjectRecord(payload, endpoint)
    const pipelineId = expectString(record.pipeline_id, endpoint, 'pipeline_id')
    const runRecord = parseRunRecord({
        ...record,
        run_id: typeof record.run_id === 'string' ? record.run_id : pipelineId,
    })
    if (!runRecord) {
        throw new ApiSchemaError(endpoint, 'Expected a valid run detail payload.')
    }
    return {
        pipeline_id: pipelineId,
        ...runRecord,
        completed_nodes: asOptionalStringArray(record.completed_nodes),
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
        started_at: typeof record.started_at === 'string' ? record.started_at : '',
        ended_at: asOptionalNullableString(record.ended_at),
        last_error: asOptionalString(record.last_error),
        token_usage: typeof record.token_usage === 'number'
            ? record.token_usage
            : record.token_usage === null
                ? null
                : undefined,
        continued_from_run_id: asOptionalNullableString(record.continued_from_run_id),
        continued_from_node: asOptionalNullableString(record.continued_from_node),
        continued_from_flow_mode: asOptionalNullableString(record.continued_from_flow_mode),
        continued_from_flow_name: asOptionalNullableString(record.continued_from_flow_name),
    }
}

export function parseRunsListResponse(payload: unknown, endpoint = '/attractor/runs'): RunsListResponse {
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
        last_run_id: asOptionalNullableString(record.last_run_id),
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
    expectSemanticEquivalence: boolean,
): Promise<SaveFlowApiResponse> {
    const response = await fetch(attractorUrl('/api/flows'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            name,
            content,
            expect_semantic_equivalence: expectSemanticEquivalence,
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

export async function fetchPreviewValidated(flowContent: string, init?: RequestInit): Promise<PreviewResponsePayload> {
    const headers = init?.headers
        ? {
            ...Object.fromEntries(new Headers(init.headers).entries()),
            'Content-Type': 'application/json',
        }
        : { 'Content-Type': 'application/json' }
    return fetchJsonWithValidation(
        attractorUrl('/preview'),
        {
            ...init,
            method: 'POST',
            headers,
            body: JSON.stringify({ flow_content: flowContent }),
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

export async function fetchPipelineStatusValidated(pipelineId: string): Promise<PipelineStatusResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}', parsePipelineStatusResponse)
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

export async function fetchPipelineQuestionsValidated(pipelineId: string): Promise<PipelineQuestionsResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/questions`)
    return fetchJsonWithValidation(url, undefined, '/attractor/pipelines/{id}/questions', parsePipelineQuestionsResponse)
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
): Promise<PipelineGraphPreviewResponse> {
    const url = attractorUrl(`/pipelines/${encodeURIComponent(pipelineId)}/graph-preview`)
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
