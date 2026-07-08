import type { NodeStatus } from '@/store'

interface BuildRunNodeStatusesArgs {
    completedNodes: string[]
    currentNodeId: string | null
    liveNodeStatuses: Record<string, NodeStatus>
    gateNodeId: string | null
    isRunActive: boolean
}

const TERMINAL_NODE_STATUSES = new Set<NodeStatus>(['success', 'failed'])

// Merges the durable checkpoint snapshot with live journal-driven statuses into
// the per-node status map the run graph renders. Precedence, lowest to highest:
// checkpoint completions, live stage statuses, the active current node, and a
// pending human gate.
export function buildRunNodeStatuses({
    completedNodes,
    currentNodeId,
    liveNodeStatuses,
    gateNodeId,
    isRunActive,
}: BuildRunNodeStatusesArgs): Record<string, NodeStatus> {
    const statuses: Record<string, NodeStatus> = {}
    for (const nodeId of completedNodes) {
        statuses[nodeId] = 'success'
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
    if (gateNodeId) {
        statuses[gateNodeId] = 'waiting'
    }
    return statuses
}
