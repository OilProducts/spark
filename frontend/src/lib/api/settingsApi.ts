import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type ExecutionMode = 'native' | 'local_container' | 'remote_worker'

export interface ExecutionPlacementValidationError {
    field: string | null
    message: string
    profile_id?: string | null
    worker_id?: string | null
}

export interface ExecutionPlacementProfile {
    id: string | null
    label: string | null
    mode: ExecutionMode
    enabled: boolean
    worker_id?: string | null
    image?: string | null
    control_project_root?: string | null
    worker_project_root?: string | null
    worker_runtime_root?: string | null
    capabilities: unknown
    metadata: Record<string, unknown>
}

export interface ExecutionPlacementWorker {
    id: string
    label: string
    base_url: string
    auth_token_env: string
    enabled: boolean
    capabilities: unknown
    metadata: Record<string, unknown>
    health: Record<string, unknown> | null
    health_error: Record<string, unknown> | null
    worker_info: Record<string, unknown> | null
    worker_info_error: Record<string, unknown> | null
    status: string | null
    versions: {
        worker_version?: string | null
        protocol_version?: string | null
        expected_protocol_version: string
    }
    protocol_compatible: boolean
    compatibility: {
        compatible: boolean
        signals: Array<Record<string, string>>
    }
}

export interface ExecutionPlacementSettings {
    execution_modes: ExecutionMode[]
    protocol: {
        expected_worker_protocol_version: string
    }
    config: {
        filename: string
        path: string
        exists: boolean
        loaded: boolean
        synthesized_native_default: boolean
    }
    default_execution_profile_id: string | null
    profiles: ExecutionPlacementProfile[]
    workers: ExecutionPlacementWorker[]
    validation_errors: ExecutionPlacementValidationError[]
}

export interface WorkspaceSettingsResponse {
    execution_placement: ExecutionPlacementSettings
}

function parseExecutionMode(value: unknown, endpoint: string): ExecutionMode {
    if (value === 'native' || value === 'local_container' || value === 'remote_worker') {
        return value
    }
    throw new ApiSchemaError(endpoint, 'Expected execution mode to be native, local_container, or remote_worker.')
}

function parseValidationError(payload: unknown, endpoint: string): ExecutionPlacementValidationError {
    const record = expectObjectRecord(payload, endpoint)
    return {
        field: asOptionalNullableString(record.field) ?? null,
        message: expectString(record.message, endpoint, 'validation_errors.message'),
        profile_id: asOptionalNullableString(record.profile_id),
        worker_id: asOptionalNullableString(record.worker_id),
    }
}

function parseProfile(payload: unknown, endpoint: string): ExecutionPlacementProfile {
    const record = expectObjectRecord(payload, endpoint)
    return {
        id: asOptionalNullableString(record.id) ?? null,
        label: asOptionalNullableString(record.label) ?? null,
        mode: parseExecutionMode(record.mode, endpoint),
        enabled: record.enabled === true,
        worker_id: asOptionalNullableString(record.worker_id),
        image: asOptionalNullableString(record.image),
        control_project_root: asOptionalNullableString(record.control_project_root),
        worker_project_root: asOptionalNullableString(record.worker_project_root),
        worker_runtime_root: asOptionalNullableString(record.worker_runtime_root),
        capabilities: record.capabilities,
        metadata: asUnknownRecord(record.metadata) ?? {},
    }
}

function parseWorker(payload: unknown, endpoint: string): ExecutionPlacementWorker {
    const record = expectObjectRecord(payload, endpoint)
    const versions = expectObjectRecord(record.versions, endpoint)
    const compatibility = expectObjectRecord(record.compatibility, endpoint)
    return {
        id: expectString(record.id, endpoint, 'workers.id'),
        label: expectString(record.label, endpoint, 'workers.label'),
        base_url: expectString(record.base_url, endpoint, 'workers.base_url'),
        auth_token_env: expectString(record.auth_token_env, endpoint, 'workers.auth_token_env'),
        enabled: record.enabled === true,
        capabilities: record.capabilities,
        metadata: asUnknownRecord(record.metadata) ?? {},
        health: asUnknownRecord(record.health),
        health_error: asUnknownRecord(record.health_error),
        worker_info: asUnknownRecord(record.worker_info),
        worker_info_error: asUnknownRecord(record.worker_info_error),
        status: asOptionalNullableString(record.status) ?? null,
        versions: {
            worker_version: asOptionalNullableString(versions.worker_version),
            protocol_version: asOptionalNullableString(versions.protocol_version),
            expected_protocol_version: expectString(versions.expected_protocol_version, endpoint, 'versions.expected_protocol_version'),
        },
        protocol_compatible: record.protocol_compatible === true,
        compatibility: {
            compatible: compatibility.compatible === true,
            signals: Array.isArray(compatibility.signals)
                ? compatibility.signals.map((entry) => {
                    const signal = asUnknownRecord(entry) ?? {}
                    const nextSignal: Record<string, string> = {}
                    Object.entries(signal).forEach(([key, value]) => {
                        if (typeof value === 'string') {
                            nextSignal[key] = value
                        }
                    })
                    return nextSignal
                })
                : [],
        },
    }
}

export function parseWorkspaceSettingsResponse(
    payload: unknown,
    endpoint = '/workspace/api/settings',
): WorkspaceSettingsResponse {
    const record = expectObjectRecord(payload, endpoint)
    const executionPlacement = expectObjectRecord(record.execution_placement, endpoint)
    const protocol = expectObjectRecord(executionPlacement.protocol, endpoint)
    const config = expectObjectRecord(executionPlacement.config, endpoint)
    return {
        execution_placement: {
            execution_modes: Array.isArray(executionPlacement.execution_modes)
                ? executionPlacement.execution_modes.map((mode) => parseExecutionMode(mode, endpoint))
                : [],
            protocol: {
                expected_worker_protocol_version: expectString(
                    protocol.expected_worker_protocol_version,
                    endpoint,
                    'protocol.expected_worker_protocol_version',
                ),
            },
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
            workers: Array.isArray(executionPlacement.workers)
                ? executionPlacement.workers.map((worker) => parseWorker(worker, endpoint))
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
