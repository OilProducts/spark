import {
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type ProjectDirectoryPickResponse =
    | {
        status: 'selected'
        directory_path: string
    }
    | {
        status: 'canceled'
    }

export interface ProjectMetadataResponse {
    name?: string
    directory?: string
    branch?: string | null
    commit?: string | null
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
}

function parseProjectRecordResponse(value: unknown, _endpoint: string): ProjectRecordResponse | null {
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
    }
}

export function parseProjectRecordListResponse(payload: unknown, endpoint = '/workspace/api/projects'): ProjectRecordResponse[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of projects.')
    }
    return payload
        .map((entry) => parseProjectRecordResponse(entry, endpoint))
        .filter((entry): entry is ProjectRecordResponse => entry !== null)
}

export function parseProjectRecordResponsePayload(payload: unknown, endpoint = '/workspace/api/projects/register'): ProjectRecordResponse {
    const record = parseProjectRecordResponse(payload, endpoint)
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

export function parseProjectDirectoryPickResponse(
    payload: unknown,
    endpoint = '/workspace/api/projects/pick-directory',
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

export async function pickProjectDirectoryValidated(): Promise<ProjectDirectoryPickResponse> {
    return fetchWorkspaceJsonValidated(
        '/projects/pick-directory',
        {
            method: 'POST',
        },
        '/workspace/api/projects/pick-directory',
        parseProjectDirectoryPickResponse,
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
