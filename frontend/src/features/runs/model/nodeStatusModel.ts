import type { NodeStatus } from '@/store'

interface BuildRunNodeStatusesArgs {
    completedNodes: string[]
    nodeOutcomes: Record<string, string>
    currentNodeId: string | null
    liveNodeStatuses: Record<string, NodeStatus>
    gateNodeId: string | null
    isRunActive: boolean
    runStatus: string | null
}

const TERMINAL_NODE_STATUSES = new Set<NodeStatus>(['success', 'failed'])

// Merges the durable checkpoint snapshot with live journal-driven statuses into
// the per-node status map the run graph renders. Precedence, lowest to highest:
// checkpoint completions, per-node checkpoint outcomes (a failed node sits in
// completed_nodes too, so failure must override), live stage statuses, the
// active current node, the failed run's terminal node, and a pending gate.
export function buildRunNodeStatuses({
    completedNodes,
    nodeOutcomes,
    currentNodeId,
    liveNodeStatuses,
    gateNodeId,
    isRunActive,
    runStatus,
}: BuildRunNodeStatusesArgs): Record<string, NodeStatus> {
    const statuses: Record<string, NodeStatus> = {}
    for (const nodeId of completedNodes) {
        statuses[nodeId] = 'success'
    }
    for (const [nodeId, outcome] of Object.entries(nodeOutcomes)) {
        if (outcome === 'fail' && statuses[nodeId]) {
            statuses[nodeId] = 'failed'
        }
    }
    for (const [nodeId, status] of Object.entries(liveNodeStatuses)) {
        if (status !== 'idle') {
            statuses[nodeId] = status
        }
    }
    if (
        isRunActive
        && currentNodeId
        && !TERMINAL_NODE_STATUSES.has(statuses[currentNodeId] ?? 'idle')
    ) {
        statuses[currentNodeId] = 'running'
    }
    if (runStatus === 'failed' && currentNodeId) {
        statuses[currentNodeId] = 'failed'
    }
    if (gateNodeId) {
        statuses[gateNodeId] = 'waiting'
    }
    return statuses
}
