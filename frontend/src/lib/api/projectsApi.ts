import {
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export interface ProjectBrowseEntryResponse {
    name: string
    path: string
    is_dir: true
}

export interface ProjectBrowseResponse {
    current_path: string
    parent_path: string | null
    roots: string[]
    entries: ProjectBrowseEntryResponse[]
}

export interface ProjectMetadataResponse {
    name?: string
    directory?: string
    branch?: string | null
    commit?: string | null
}

export interface ProjectChatModelMetadataResponse {
    llm_profile?: string | null
    provider: string
    id: string
    display: string
    is_default: boolean
    supported_reasoning_efforts: string[]
    reasoning_unverified?: boolean
    default_reasoning_effort?: string | null
}

export interface ProjectChatModelsResponse {
    models: ProjectChatModelMetadataResponse[]
    provider_reasoning_efforts?: Record<string, string[]>
    providers: {
        [provider: string]: ProjectChatModelProviderStatusResponse
        codex: ProjectChatModelProviderStatusResponse
    }
}

export interface ProjectChatModelProviderStatusResponse {
    status: 'available' | 'unavailable'
    error: string | null
}

export interface ProjectDeleteResponse {
    status: 'deleted'
    project_id: string
    project_path: string
    display_name: string
}

export interface ProjectRecordResponse {
    project_id: string
    project_path: string
    display_name: string
    created_at: string
    last_opened_at: string
    last_accessed_at?: string | null
    is_favorite: boolean
    active_conversation_id?: string | null
    execution_profile_id?: string | null
}

function parseProjectRecordResponse(value: unknown): ProjectRecordResponse | null {
    const record = value && typeof value === 'object' && !Array.isArray(value)
        ? value as Record<string, unknown>
        : null
    if (
        !record
        || typeof record.project_id !== 'string'
        || typeof record.project_path !== 'string'
        || typeof record.display_name !== 'string'
    ) {
        return null
    }
    return {
        project_id: record.project_id,
        project_path: record.project_path,
        display_name: record.display_name,
        created_at: typeof record.created_at === 'string' ? record.created_at : '',
        last_opened_at: typeof record.last_opened_at === 'string' ? record.last_opened_at : '',
        last_accessed_at: asOptionalNullableString(record.last_accessed_at),
        is_favorite: record.is_favorite === true,
        active_conversation_id: asOptionalNullableString(record.active_conversation_id),
        execution_profile_id: asOptionalNullableString(record.execution_profile_id),
    }
}

export function parseProjectRecordListResponse(payload: unknown, endpoint = '/workspace/api/projects'): ProjectRecordResponse[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of projects.')
    }
    return payload
        .map((entry) => parseProjectRecordResponse(entry))
        .filter((entry): entry is ProjectRecordResponse => entry !== null)
}

export function parseProjectRecordResponsePayload(payload: unknown, endpoint = '/workspace/api/projects/register'): ProjectRecordResponse {
    const record = parseProjectRecordResponse(payload)
    if (!record) {
        throw new ApiSchemaError(endpoint, 'Expected a project record response.')
    }
    return record
}

export function parseProjectDeleteResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects',
): ProjectDeleteResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        status: expectString(record.status, endpoint, 'status') === 'deleted' ? 'deleted' : 'deleted',
        project_id: expectString(record.project_id, endpoint, 'project_id'),
        project_path: expectString(record.project_path, endpoint, 'project_path'),
        display_name: expectString(record.display_name, endpoint, 'display_name'),
    }
}

function parseProjectBrowseEntryResponse(
    payload: unknown,
    endpoint: string,
): ProjectBrowseEntryResponse {
    const record = expectObjectRecord(payload, endpoint)
    const isDir = record.is_dir
    if (isDir !== true) {
        throw new ApiSchemaError(endpoint, 'Expected browse entry "is_dir" to be true.')
    }
    return {
        name: expectString(record.name, endpoint, 'name'),
        path: expectString(record.path, endpoint, 'path'),
        is_dir: true,
    }
}

export function parseProjectBrowseResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/browse',
): ProjectBrowseResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (!Array.isArray(record.entries)) {
        throw new ApiSchemaError(endpoint, 'Expected "entries" to be an array.')
    }
    if (record.roots !== undefined && !Array.isArray(record.roots)) {
        throw new ApiSchemaError(endpoint, 'Expected "roots" to be an array when present.')
    }
    const parentPath = record.parent_path
    if (parentPath !== null && typeof parentPath !== 'string') {
        throw new ApiSchemaError(endpoint, 'Expected "parent_path" to be a string or null.')
    }
    return {
        current_path: expectString(record.current_path, endpoint, 'current_path'),
        parent_path: parentPath,
        roots: Array.isArray(record.roots)
            ? record.roots.map((entry) => expectString(entry, endpoint, 'roots'))
            : [],
        entries: record.entries.map((entry) => parseProjectBrowseEntryResponse(entry, endpoint)),
    }
}

export function parseProjectMetadataResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/metadata',
): ProjectMetadataResponse {
    const record = expectObjectRecord(payload, endpoint)
    return {
        name: asOptionalString(record.name),
        directory: asOptionalString(record.directory),
        branch: asOptionalString(record.branch),
        commit: asOptionalString(record.commit),
    }
}

function parseProjectChatModelMetadataResponse(
    payload: unknown,
    _endpoint: string,
): ProjectChatModelMetadataResponse | null {
    const record = payload && typeof payload === 'object' && !Array.isArray(payload)
        ? payload as Record<string, unknown>
        : null
    if (!record || typeof record.id !== 'string') {
        return null
    }
    return {
        id: record.id,
        ...(typeof record.llm_profile === 'string' ? { llm_profile: record.llm_profile } : {}),
        provider: typeof record.provider === 'string' && record.provider.trim().length > 0
            ? record.provider
            : 'codex',
        display: typeof record.display === 'string' && record.display.trim().length > 0
            ? record.display
            : record.id,
        is_default: record.is_default === true,
        supported_reasoning_efforts: Array.isArray(record.supported_reasoning_efforts)
            ? record.supported_reasoning_efforts
                .filter((entry): entry is string => typeof entry === 'string')
                .filter((entry) => entry.trim().length > 0)
            : [],
        default_reasoning_effort: asOptionalNullableString(record.default_reasoning_effort),
        ...(record.reasoning_unverified === true ? { reasoning_unverified: true } : {}),
    }
}

export function parseProjectChatModelsResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/chat-models',
): ProjectChatModelsResponse {
    const record = expectObjectRecord(payload, endpoint)
    if (!Array.isArray(record.models)) {
        throw new ApiSchemaError(endpoint, 'Expected "models" to be an array.')
    }
    const providers = expectObjectRecord(record.providers, endpoint)
    expectObjectRecord(providers.codex, endpoint)
    const statuses: ProjectChatModelsResponse['providers'] = Object.fromEntries(Object.entries(providers).map(([provider, value]) => {
        const entry = expectObjectRecord(value, endpoint)
        if (entry.status !== 'available' && entry.status !== 'unavailable') {
            throw new ApiSchemaError(endpoint, 'Expected provider status to be "available" or "unavailable".')
        }
        if (entry.error !== null && typeof entry.error !== 'string') {
            throw new ApiSchemaError(endpoint, 'Expected provider error to be a string or null.')
        }
        return [provider, { status: entry.status, error: entry.error }]
    })) as ProjectChatModelsResponse['providers']
    return {
        ...(record.provider_reasoning_efforts && typeof record.provider_reasoning_efforts === 'object' ? {
            provider_reasoning_efforts: Object.fromEntries(Object.entries(record.provider_reasoning_efforts).map(([provider, efforts]) =>
                [provider, Array.isArray(efforts) ? efforts.filter((effort): effort is string => typeof effort === 'string' && effort.trim().length > 0) : []])),
        } : {}),
        models: record.models
            .map((entry) => parseProjectChatModelMetadataResponse(entry, endpoint))
            .filter((entry): entry is ProjectChatModelMetadataResponse => entry !== null),
        providers: statuses,
    }
}

export async function deleteProjectValidated(projectPath: string): Promise<ProjectDeleteResponse> {
    return fetchWorkspaceJsonValidated(
        `/projects?project_path=${encodeURIComponent(projectPath)}`,
        {
            method: 'DELETE',
        },
        '/workspace/api/projects',
        parseProjectDeleteResponse,
    )
}

export async function fetchProjectRegistryValidated(): Promise<ProjectRecordResponse[]> {
    return fetchWorkspaceJsonValidated('/projects', undefined, '/workspace/api/projects', parseProjectRecordListResponse)
}

export async function registerProjectValidated(projectPath: string): Promise<ProjectRecordResponse> {
    return fetchWorkspaceJsonValidated(
        '/projects/register',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_path: projectPath }),
        },
        '/workspace/api/projects/register',
        parseProjectRecordResponsePayload,
    )
}

export async function updateProjectStateValidated(payload: {
    expected_revision?: string
    project_path: string
    is_favorite?: boolean | null
    last_accessed_at?: string | null
    active_conversation_id?: string | null
    execution_profile_id?: string | null
}): Promise<ProjectRecordResponse> {
    return fetchWorkspaceJsonValidated(
        '/projects/state',
        {
            method: 'PATCH',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/projects/state',
        parseProjectRecordResponsePayload,
    )
}

export async function fetchProjectBrowseValidated(path?: string): Promise<ProjectBrowseResponse> {
    const query = typeof path === 'string' ? `?path=${encodeURIComponent(path)}` : ''
    return fetchWorkspaceJsonValidated(
        `/projects/browse${query}`,
        undefined,
        '/workspace/api/projects/browse',
        parseProjectBrowseResponse,
    )
}

export async function fetchProjectMetadataValidated(directory: string): Promise<ProjectMetadataResponse> {
    return fetchWorkspaceJsonValidated(
        `/projects/metadata?directory=${encodeURIComponent(directory)}`,
        undefined,
        '/workspace/api/projects/metadata',
        parseProjectMetadataResponse,
    )
}

export async function fetchProjectChatModelsValidated(projectPath: string | null): Promise<ProjectChatModelsResponse> {
    return fetchWorkspaceJsonValidated(
        projectPath ? `/projects/chat-models?project_path=${encodeURIComponent(projectPath)}` : '/projects/chat-models',
        undefined,
        '/workspace/api/projects/chat-models',
        parseProjectChatModelsResponse,
    )
}
