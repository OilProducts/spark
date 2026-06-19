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
