import { encodeFlowPath } from '@/lib/flowPaths'
import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type FlowExecutionLockScope = 'project'
export type FlowExecutionLockConflictPolicy = 'queue'

export interface FlowExecutionLockResponse {
    scope: FlowExecutionLockScope
    key: string
    conflict_policy: FlowExecutionLockConflictPolicy
}

export interface WorkspaceFlowResponse {
    name: string
    title: string
    description: string
    execution_lock?: FlowExecutionLockResponse | null
    graph_label?: string | null
    graph_goal?: string | null
    node_count?: number
    edge_count?: number
    features?: {
        has_human_gate: boolean
        has_manager_loop: boolean
    }
}

function parseFlowExecutionLockScope(
    value: unknown,
    endpoint: string,
    fieldName: string,
): FlowExecutionLockScope {
    if (value === 'project') {
        return value
    }
    throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be project.`)
}

function parseFlowExecutionLockConflictPolicy(
    value: unknown,
    endpoint: string,
    fieldName: string,
): FlowExecutionLockConflictPolicy {
    if (value === 'queue') {
        return value
    }
    throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be queue.`)
}

function parseFlowExecutionLock(
    value: unknown,
    endpoint: string,
    fieldName: string,
): FlowExecutionLockResponse | null {
    if (value === null || value === undefined) {
        return null
    }
    const record = expectObjectRecord(value, endpoint)
    return {
        scope: parseFlowExecutionLockScope(record.scope, endpoint, `${fieldName}.scope`),
        key: expectString(record.key, endpoint, `${fieldName}.key`),
        conflict_policy: parseFlowExecutionLockConflictPolicy(
            record.conflict_policy,
            endpoint,
            `${fieldName}.conflict_policy`,
        ),
    }
}

export function parseWorkspaceFlowResponse(value: unknown, endpoint: string): WorkspaceFlowResponse {
    const record = expectObjectRecord(value, endpoint)
    const nodeCount = typeof record.node_count === 'number' ? record.node_count : undefined
    const edgeCount = typeof record.edge_count === 'number' ? record.edge_count : undefined
    const graphLabel = asOptionalNullableString(record.graph_label)
    const graphGoal = asOptionalNullableString(record.graph_goal)
    const featuresRecord = asUnknownRecord(record.features)
    const features = featuresRecord
        && typeof featuresRecord.has_human_gate === 'boolean'
        && typeof featuresRecord.has_manager_loop === 'boolean'
        ? {
            has_human_gate: featuresRecord.has_human_gate,
            has_manager_loop: featuresRecord.has_manager_loop,
        }
        : undefined
    return {
        name: expectString(record.name, endpoint, 'name'),
        title: expectString(record.title, endpoint, 'title'),
        description: expectString(record.description, endpoint, 'description'),
        execution_lock: parseFlowExecutionLock(record.execution_lock, endpoint, 'execution_lock'),
        graph_label: graphLabel,
        graph_goal: graphGoal,
        node_count: nodeCount,
        edge_count: edgeCount,
        features,
    }
}

export function parseWorkspaceFlowListResponse(payload: unknown, endpoint = '/workspace/api/flows'): WorkspaceFlowResponse[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of flows.')
    }
    return payload
        .map((entry) => parseWorkspaceFlowResponse(entry, endpoint))
        .filter((entry): entry is WorkspaceFlowResponse => entry !== null)
}

function projectQuery(projectPath: string | null): string {
    return projectPath ? `?project_path=${encodeURIComponent(projectPath)}` : ''
}

export async function fetchWorkspaceFlowListValidated(projectPath: string | null): Promise<WorkspaceFlowResponse[]> {
    return fetchWorkspaceJsonValidated(
        `/flows${projectQuery(projectPath)}`,
        undefined,
        '/workspace/api/flows',
        parseWorkspaceFlowListResponse,
    )
}

export async function fetchWorkspaceFlowValidated(
    flowName: string,
    projectPath: string | null,
): Promise<WorkspaceFlowResponse> {
    return fetchWorkspaceJsonValidated(
        `/flows/${encodeFlowPath(flowName)}${projectQuery(projectPath)}`,
        undefined,
        '/workspace/api/flows/{flow_name}',
        parseWorkspaceFlowResponse,
    )
}
