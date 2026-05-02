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
    provider: string
    id: string
    display: string
    is_default: boolean
    supported_reasoning_efforts: string[]
    default_reasoning_effort?: string | null
}

export interface ProjectChatModelsResponse {
    models: ProjectChatModelMetadataResponse[]
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
                .filter((entry) => ['low', 'medium', 'high', 'xhigh'].includes(entry))
            : [],
        default_reasoning_effort: asOptionalNullableString(record.default_reasoning_effort),
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
    return {
        models: record.models
            .map((entry) => parseProjectChatModelMetadataResponse(entry, endpoint))
            .filter((entry): entry is ProjectChatModelMetadataResponse => entry !== null),
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
    project_path: string
    is_favorite?: boolean | null
    last_accessed_at?: string | null
    active_conversation_id?: string | null
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

export async function fetchProjectChatModelsValidated(projectPath: string): Promise<ProjectChatModelsResponse> {
    return fetchWorkspaceJsonValidated(
        `/projects/chat-models?project_path=${encodeURIComponent(projectPath)}`,
        undefined,
        '/workspace/api/projects/chat-models',
        parseProjectChatModelsResponse,
    )
}
