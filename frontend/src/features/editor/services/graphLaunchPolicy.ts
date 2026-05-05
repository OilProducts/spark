import {
    fetchWorkspaceFlowValidated,
    updateWorkspaceFlowLaunchPolicyValidated,
    type FlowExecutionLockResponse,
    type FlowLaunchPolicy,
} from '@/lib/workspaceClient'

export type { FlowExecutionLockResponse, FlowLaunchPolicy }

export const loadGraphLaunchPolicy = (flowName: string) => (
    fetchWorkspaceFlowValidated(flowName, 'human')
)

export const saveGraphLaunchPolicy = updateWorkspaceFlowLaunchPolicyValidated
