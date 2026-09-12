import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type ExecutionMode = 'native' | 'local_container'

export interface ExecutionPlacementValidationError {
    field: string | null
    message: string
    profile_id?: string | null
}

export interface ExecutionPlacementProfile {
    id: string | null
    label: string | null
    mode: ExecutionMode
    enabled: boolean
    image?: string | null
    capabilities: unknown
    metadata: Record<string, unknown>
}

export interface ExecutionPlacementSettings {
    execution_modes: ExecutionMode[]
    config: {
        filename: string
        path: string
        exists: boolean
        loaded: boolean
        synthesized_native_default: boolean
    }
    default_execution_profile_id: string | null
    profiles: ExecutionPlacementProfile[]
    validation_errors: ExecutionPlacementValidationError[]
}

export interface WorkspaceSettingsResponse {
    execution_placement: ExecutionPlacementSettings
}

function parseExecutionMode(value: unknown, endpoint: string): ExecutionMode {
    if (value === 'native' || value === 'local_container') {
        return value
    }
    throw new ApiSchemaError(endpoint, 'Expected execution mode to be native or local_container.')
}

function parseValidationError(payload: unknown, endpoint: string): ExecutionPlacementValidationError {
    const record = expectObjectRecord(payload, endpoint)
    return {
        field: asOptionalNullableString(record.field) ?? null,
        message: expectString(record.message, endpoint, 'validation_errors.message'),
        profile_id: asOptionalNullableString(record.profile_id),
    }
}

function parseProfile(payload: unknown, endpoint: string): ExecutionPlacementProfile {
    const record = expectObjectRecord(payload, endpoint)
    return {
        id: asOptionalNullableString(record.id) ?? null,
        label: asOptionalNullableString(record.label) ?? null,
        mode: parseExecutionMode(record.mode, endpoint),
        enabled: record.enabled === true,
        image: asOptionalNullableString(record.image),
        capabilities: record.capabilities,
        metadata: asUnknownRecord(record.metadata) ?? {},
    }
}

export function parseWorkspaceSettingsResponse(
    payload: unknown,
    endpoint = '/workspace/api/settings',
): WorkspaceSettingsResponse {
    const record = expectObjectRecord(payload, endpoint)
    const executionPlacement = expectObjectRecord(record.execution_placement, endpoint)
    const config = expectObjectRecord(executionPlacement.config, endpoint)
    return {
        execution_placement: {
            execution_modes: Array.isArray(executionPlacement.execution_modes)
                ? executionPlacement.execution_modes.map((mode) => parseExecutionMode(mode, endpoint))
                : [],
            config: {
                filename: expectString(config.filename, endpoint, 'config.filename'),
                path: expectString(config.path, endpoint, 'config.path'),
                exists: config.exists === true,
                loaded: config.loaded === true,
                synthesized_native_default: config.synthesized_native_default === true,
            },
            default_execution_profile_id: asOptionalNullableString(executionPlacement.default_execution_profile_id) ?? null,
            profiles: Array.isArray(executionPlacement.profiles)
                ? executionPlacement.profiles.map((profile) => parseProfile(profile, endpoint))
                : [],
            validation_errors: Array.isArray(executionPlacement.validation_errors)
                ? executionPlacement.validation_errors.map((error) => parseValidationError(error, endpoint))
                : [],
        },
    }
}

export async function fetchWorkspaceSettingsValidated(): Promise<WorkspaceSettingsResponse> {
    return fetchWorkspaceJsonValidated(
        '/settings',
        undefined,
        '/workspace/api/settings',
        parseWorkspaceSettingsResponse,
    )
}

export interface RuntimeSettings {
    runs_dir: string | null
    flows_dir: string | null
    ui_dir: string | null
    project_roots: string[]
}

export interface RuntimeSettingsView {
    scope: 'workspace'
    revision: string
    stored: RuntimeSettings
    effective: RuntimeSettings
    restart_fields: string[]
}

function parseRuntimeSettings(value: unknown, endpoint: string): RuntimeSettings {
    const record = expectObjectRecord(value, endpoint)
    const path = (key: string) => record[key] == null ? null : expectString(record[key], endpoint, key)
    if (!Array.isArray(record.project_roots)) throw new ApiSchemaError(endpoint, 'Expected project roots.')
    return {
        runs_dir: path('runs_dir'), flows_dir: path('flows_dir'), ui_dir: path('ui_dir'),
        project_roots: record.project_roots.map((entry) => expectString(entry, endpoint, 'project_roots')),
    }
}

function parseRuntimeView(payload: unknown, endpoint: string): RuntimeSettingsView {
    const record = expectObjectRecord(expectObjectRecord(payload, endpoint).runtime, endpoint)
    if (record.scope !== 'workspace') throw new ApiSchemaError(endpoint, 'Expected workspace scope.')
    return {
        scope: 'workspace', revision: expectString(record.revision, endpoint, 'revision'),
        stored: parseRuntimeSettings(record.stored, endpoint), effective: parseRuntimeSettings(record.effective, endpoint),
        restart_fields: Array.isArray(record.restart_fields) ? record.restart_fields.map((value) => expectString(value, endpoint, 'restart_fields')) : [],
    }
}

export async function fetchRuntimeSettings(): Promise<RuntimeSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', undefined, '/workspace/api/settings', parseRuntimeView)
}

export async function saveRuntimeSettings(revision: string, value: RuntimeSettings): Promise<RuntimeSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', {
        method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: revision, section: 'runtime', value }),
    }, '/workspace/api/settings', parseRuntimeView)
}

export interface ModelSettings {
    provider: string | null
    llm_profile: string | null
    model: string | null
    reasoning_effort: string | null
}

export interface ModelSettingsView {
    scope: 'workspace' | 'project' | 'conversation'
    revision: string
    stored: ModelSettings | null
    effective: ModelSettings
    source: 'workspace' | 'project' | 'conversation'
}

export function parseModelSettingsView(payload: unknown, endpoint: string): ModelSettingsView {
    const record = expectObjectRecord(expectObjectRecord(payload, endpoint).models, endpoint)
    const group = (value: unknown): ModelSettings => {
        const fields = expectObjectRecord(value, endpoint)
        const field = (key: string) => fields[key] == null ? null : expectString(fields[key], endpoint, key)
        const result = { provider: field('provider'), llm_profile: field('llm_profile'), model: field('model'), reasoning_effort: field('reasoning_effort') }
        if (!!result.provider === !!result.llm_profile) throw new ApiSchemaError(endpoint, 'Expected exactly one provider or profile.')
        return result
    }
    const scope = record.scope
    const source = record.source
    if (scope !== 'workspace' && scope !== 'project' && scope !== 'conversation') throw new ApiSchemaError(endpoint, 'Invalid settings scope.')
    if (source !== 'workspace' && source !== 'project' && source !== 'conversation') throw new ApiSchemaError(endpoint, 'Invalid settings source.')
    return { scope, source, revision: expectString(record.revision, endpoint, 'revision'), stored: record.stored == null ? null : group(record.stored), effective: group(record.effective) }
}

export function fetchModelSettings(projectPath?: string): Promise<ModelSettingsView> {
    return fetchWorkspaceJsonValidated(`/settings${projectPath ? `?project_path=${encodeURIComponent(projectPath)}` : ''}`, undefined,
        '/workspace/api/settings', parseModelSettingsView)
}

export function saveModelSettings(revision: string, value: ModelSettings | null, projectPath?: string): Promise<ModelSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', {
        method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: revision, section: projectPath ? 'project_models' : 'models',
            value: projectPath ? { project_path: projectPath, model_settings: value } : value }),
    }, '/workspace/api/settings', parseModelSettingsView)
}

export function fetchProjectExecutionSettings(projectPath: string): Promise<{ revision: string, stored: string | null }> {
    return fetchWorkspaceJsonValidated(`/settings?project_path=${encodeURIComponent(projectPath)}`, undefined, '/workspace/api/settings', (payload, endpoint) => {
        const record = expectObjectRecord(expectObjectRecord(payload, endpoint).execution, endpoint)
        return { revision: expectString(record.revision, endpoint, 'revision'), stored: record.stored == null ? null : expectString(record.stored, endpoint, 'stored') }
    })
}

export interface ConnectionSettings {
    server_host: string | null
    server_port: number | null
    client_api_base_url: string | null
}
export interface ConnectionSettingsView {
    revision: string
    stored: ConnectionSettings
    effective: ConnectionSettings
}
function parseConnectionView(payload: unknown, endpoint: string): ConnectionSettingsView {
    const view = expectObjectRecord(expectObjectRecord(payload, endpoint).connections, endpoint)
    const parse = (value: unknown): ConnectionSettings => {
        const fields = expectObjectRecord(value, endpoint)
        const port = fields.server_port
        if (port != null && (typeof port !== 'number' || !Number.isInteger(port) || port < 0 || port > 65535)) throw new ApiSchemaError(endpoint, 'Invalid server port.')
        return { server_host: fields.server_host == null ? null : expectString(fields.server_host, endpoint, 'server_host'),
            server_port: port == null ? null : port as number,
            client_api_base_url: fields.client_api_base_url == null ? null : expectString(fields.client_api_base_url, endpoint, 'client_api_base_url') }
    }
    return { revision: expectString(view.revision, endpoint, 'revision'), stored: parse(view.stored), effective: parse(view.effective) }
}
export function fetchConnectionSettings(): Promise<ConnectionSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', undefined, '/workspace/api/settings', parseConnectionView)
}
export function saveConnectionSettings(revision: string, value: ConnectionSettings): Promise<ConnectionSettingsView> {
    return fetchWorkspaceJsonValidated('/settings', { method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expected_revision: revision, section: 'connections', value }) }, '/workspace/api/settings', parseConnectionView)
}
