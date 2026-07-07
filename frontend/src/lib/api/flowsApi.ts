import { encodeFlowPath } from '@/lib/flowPaths'
import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated, fetchWorkspaceTextValidated } from './apiClient'

export type FlowLaunchPolicy = 'agent_requestable' | 'trigger_only' | 'disabled'
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
    launch_policy: FlowLaunchPolicy | null
    effective_launch_policy: FlowLaunchPolicy
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

export interface WorkspaceFlowLaunchPolicyResponse {
    name: string
    launch_policy: FlowLaunchPolicy | null
    effective_launch_policy: FlowLaunchPolicy
    execution_lock?: FlowExecutionLockResponse | null
    allowed_launch_policies?: FlowLaunchPolicy[]
    allowed_execution_lock_scopes?: FlowExecutionLockScope[]
    allowed_execution_lock_conflict_policies?: FlowExecutionLockConflictPolicy[]
}

function parseFlowLaunchPolicy(value: unknown, endpoint: string, fieldName: string, allowNull = false): FlowLaunchPolicy | null {
    if (value === null) {
        if (allowNull) {
            return null
        }
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be a flow launch policy string.`)
    }
    if (typeof value !== 'string') {
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be a flow launch policy string.`)
    }
    if (value === 'agent_requestable' || value === 'trigger_only' || value === 'disabled') {
        return value
    }
    throw new ApiSchemaError(
        endpoint,
        `Expected "${fieldName}" to be agent_requestable|trigger_only|disabled; got "${value}".`,
    )
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
        launch_policy: parseFlowLaunchPolicy(record.launch_policy, endpoint, 'launch_policy', true),
        effective_launch_policy: parseFlowLaunchPolicy(record.effective_launch_policy, endpoint, 'effective_launch_policy')!,
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

export function parseWorkspaceFlowRawResponse(payload: unknown, endpoint = '/workspace/api/flows/{flow_name}/raw'): string {
    if (typeof payload !== 'string') {
        throw new ApiSchemaError(endpoint, 'Expected raw YAML text response.')
    }
    return payload
}

export function parseWorkspaceFlowLaunchPolicyResponse(
    payload: unknown,
    endpoint = '/workspace/api/flows/{flow_name}/launch-policy',
): WorkspaceFlowLaunchPolicyResponse {
    const record = expectObjectRecord(payload, endpoint)
    const allowedRaw = Array.isArray(record.allowed_launch_policies) ? record.allowed_launch_policies : undefined
    const allowedLaunchPolicies = allowedRaw?.map((entry) => parseFlowLaunchPolicy(entry, endpoint, 'allowed_launch_policies'))
        .filter((entry): entry is FlowLaunchPolicy => entry !== null)
    const allowedScopesRaw = Array.isArray(record.allowed_execution_lock_scopes)
        ? record.allowed_execution_lock_scopes
        : undefined
    const allowedConflictPoliciesRaw = Array.isArray(record.allowed_execution_lock_conflict_policies)
        ? record.allowed_execution_lock_conflict_policies
        : undefined
    return {
        name: expectString(record.name, endpoint, 'name'),
        launch_policy: parseFlowLaunchPolicy(record.launch_policy, endpoint, 'launch_policy', true),
        effective_launch_policy: parseFlowLaunchPolicy(record.effective_launch_policy, endpoint, 'effective_launch_policy')!,
        execution_lock: parseFlowExecutionLock(record.execution_lock, endpoint, 'execution_lock'),
        allowed_launch_policies: allowedLaunchPolicies,
        allowed_execution_lock_scopes: allowedScopesRaw?.map((entry) => (
            parseFlowExecutionLockScope(entry, endpoint, 'allowed_execution_lock_scopes')
        )),
        allowed_execution_lock_conflict_policies: allowedConflictPoliciesRaw?.map((entry) => (
            parseFlowExecutionLockConflictPolicy(entry, endpoint, 'allowed_execution_lock_conflict_policies')
        )),
    }
}

export async function fetchWorkspaceFlowListValidated(surface: 'human' | 'agent' = 'human'): Promise<WorkspaceFlowResponse[]> {
    return fetchWorkspaceJsonValidated(
        `/flows?surface=${encodeURIComponent(surface)}`,
        undefined,
        '/workspace/api/flows',
        parseWorkspaceFlowListResponse,
    )
}

export async function fetchWorkspaceFlowValidated(
    flowName: string,
    surface: 'human' | 'agent' = 'human',
): Promise<WorkspaceFlowResponse> {
    return fetchWorkspaceJsonValidated(
        `/flows/${encodeFlowPath(flowName)}?surface=${encodeURIComponent(surface)}`,
        undefined,
        '/workspace/api/flows/{flow_name}',
        parseWorkspaceFlowResponse,
    )
}

export async function fetchWorkspaceFlowRawValidated(
    flowName: string,
    surface: 'human' | 'agent' = 'human',
): Promise<string> {
    return fetchWorkspaceTextValidated(
        `/flows/${encodeFlowPath(flowName)}/raw?surface=${encodeURIComponent(surface)}`,
        undefined,
        '/workspace/api/flows/{flow_name}/raw',
        parseWorkspaceFlowRawResponse,
    )
}

export async function updateWorkspaceFlowLaunchPolicyValidated(
    flowName: string,
    payload: {
        launch_policy: FlowLaunchPolicy
        execution_lock?: FlowExecutionLockResponse | null
    },
): Promise<WorkspaceFlowLaunchPolicyResponse> {
    return fetchWorkspaceJsonValidated(
        `/flows/${encodeFlowPath(flowName)}/launch-policy`,
        {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/flows/{flow_name}/launch-policy',
        parseWorkspaceFlowLaunchPolicyResponse,
    )
}
