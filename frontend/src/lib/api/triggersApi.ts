import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type TriggerSourceType = 'schedule' | 'poll' | 'webhook' | 'workspace_event' | 'flow_event'

export interface TriggerActionResponse {
    flow_name: string
    project_path?: string | null
    static_context?: Record<string, unknown>
}

export interface TriggerStateResponse {
    last_fired_at?: string | null
    last_result?: string | null
    last_error?: string | null
    next_run_at?: string | null
    recent_history: Array<{
        timestamp: string
        status: string
        message: string
        run_id?: string | null
        dedupe_key?: string | null
    }>
}

export interface TriggerResponse {
    id: string
    name: string
    enabled: boolean
    protected: boolean
    source_type: TriggerSourceType
    created_at: string
    updated_at: string
    action: TriggerActionResponse
    source: Record<string, unknown>
    state: TriggerStateResponse
    webhook_secret?: string | null
}

function parseTriggerStateResponse(value: unknown, endpoint: string): TriggerStateResponse {
    const record = expectObjectRecord(value, endpoint)
    const recentHistory = Array.isArray(record.recent_history)
        ? record.recent_history
            .map((entry) => asUnknownRecord(entry))
            .filter((entry): entry is Record<string, unknown> => entry !== null)
            .filter((entry) => typeof entry.timestamp === 'string' && typeof entry.status === 'string' && typeof entry.message === 'string')
            .map((entry) => ({
                timestamp: String(entry.timestamp),
                status: String(entry.status),
                message: String(entry.message),
                run_id: asOptionalNullableString(entry.run_id),
                dedupe_key: asOptionalNullableString(entry.dedupe_key),
            }))
        : []
    return {
        last_fired_at: asOptionalNullableString(record.last_fired_at),
        last_result: asOptionalNullableString(record.last_result),
        last_error: asOptionalNullableString(record.last_error),
        next_run_at: asOptionalNullableString(record.next_run_at),
        recent_history: recentHistory,
    }
}

export function parseTriggerResponse(payload: unknown, endpoint = '/workspace/api/triggers'): TriggerResponse {
    const record = expectObjectRecord(payload, endpoint)
    const sourceType = expectString(record.source_type, endpoint, 'source_type')
    if (
        sourceType !== 'schedule'
        && sourceType !== 'poll'
        && sourceType !== 'webhook'
        && sourceType !== 'workspace_event'
        && sourceType !== 'flow_event'
    ) {
        throw new ApiSchemaError(endpoint, `Unsupported trigger source_type "${sourceType}".`)
    }
    const actionRecord = expectObjectRecord(record.action, endpoint)
    return {
        id: expectString(record.id, endpoint, 'id'),
        name: expectString(record.name, endpoint, 'name'),
        enabled: record.enabled === true,
        protected: record.protected === true,
        source_type: sourceType,
        created_at: expectString(record.created_at, endpoint, 'created_at'),
        updated_at: expectString(record.updated_at, endpoint, 'updated_at'),
        action: {
            flow_name: expectString(actionRecord.flow_name, endpoint, 'action.flow_name'),
            project_path: asOptionalNullableString(actionRecord.project_path),
            static_context: asUnknownRecord(actionRecord.static_context) ?? {},
        },
        source: asUnknownRecord(record.source) ?? {},
        state: parseTriggerStateResponse(record.state, endpoint),
        webhook_secret: asOptionalNullableString(record.webhook_secret),
    }
}

export function parseTriggerListResponse(payload: unknown, endpoint = '/workspace/api/triggers'): TriggerResponse[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of triggers.')
    }
    return payload.map((entry) => parseTriggerResponse(entry, endpoint))
}

export async function fetchTriggerListValidated(): Promise<TriggerResponse[]> {
    return fetchWorkspaceJsonValidated('/triggers', undefined, '/workspace/api/triggers', parseTriggerListResponse)
}

export async function fetchTriggerValidated(triggerId: string): Promise<TriggerResponse> {
    return fetchWorkspaceJsonValidated(
        `/triggers/${encodeURIComponent(triggerId)}`,
        undefined,
        '/workspace/api/triggers/{id}',
        parseTriggerResponse,
    )
}

export async function createTriggerValidated(payload: {
    name: string
    enabled: boolean
    source_type: TriggerSourceType
    action: Record<string, unknown>
    source: Record<string, unknown>
}): Promise<TriggerResponse> {
    return fetchWorkspaceJsonValidated(
        '/triggers',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/triggers',
        parseTriggerResponse,
    )
}

export async function updateTriggerValidated(
    triggerId: string,
    payload: {
        name?: string
        enabled?: boolean
        action?: Record<string, unknown>
        source?: Record<string, unknown>
        regenerate_webhook_secret?: boolean
    },
): Promise<TriggerResponse> {
    return fetchWorkspaceJsonValidated(
        `/triggers/${encodeURIComponent(triggerId)}`,
        {
            method: 'PATCH',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/triggers/{id}',
        parseTriggerResponse,
    )
}

export async function deleteTriggerValidated(triggerId: string): Promise<{ status: 'deleted'; id: string }> {
    return fetchWorkspaceJsonValidated(
        `/triggers/${encodeURIComponent(triggerId)}`,
        {
            method: 'DELETE',
        },
        '/workspace/api/triggers/{id}',
        (payload, endpoint) => {
            const record = expectObjectRecord(payload, endpoint)
            return {
                status: expectString(record.status, endpoint, 'status') === 'deleted' ? 'deleted' : 'deleted',
                id: expectString(record.id, endpoint, 'id'),
            }
        },
    )
}
