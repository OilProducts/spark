import {
    ApiSchemaError,
    asOptionalNullableString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
} from './shared'
import { fetchWorkspaceJsonValidated, fetchWorkspaceTextValidated } from './apiClient'

export type FlowLaunchPolicy = 'agent_requestable' | 'trigger_only' | 'disabled'

export interface WorkspaceFlowResponse {
    name: string
    title: string
    description: string
    launch_policy: FlowLaunchPolicy | null
    effective_launch_policy: FlowLaunchPolicy
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
    allowed_launch_policies?: FlowLaunchPolicy[]
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
        throw new ApiSchemaError(endpoint, 'Expected raw DOT text response.')
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
    return {
        name: expectString(record.name, endpoint, 'name'),
        launch_policy: parseFlowLaunchPolicy(record.launch_policy, endpoint, 'launch_policy', true),
        effective_launch_policy: parseFlowLaunchPolicy(record.effective_launch_policy, endpoint, 'effective_launch_policy')!,
        allowed_launch_policies: allowedLaunchPolicies,
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
        `/flows/${encodeURIComponent(flowName)}?surface=${encodeURIComponent(surface)}`,
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
        `/flows/${encodeURIComponent(flowName)}/raw?surface=${encodeURIComponent(surface)}`,
        undefined,
        '/workspace/api/flows/{flow_name}/raw',
        parseWorkspaceFlowRawResponse,
    )
}

export async function updateWorkspaceFlowLaunchPolicyValidated(
    flowName: string,
    launchPolicy: FlowLaunchPolicy,
): Promise<WorkspaceFlowLaunchPolicyResponse> {
    return fetchWorkspaceJsonValidated(
        `/flows/${encodeURIComponent(flowName)}/launch-policy`,
        {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                launch_policy: launchPolicy,
            }),
        },
        '/workspace/api/flows/{flow_name}/launch-policy',
        parseWorkspaceFlowLaunchPolicyResponse,
    )
}
