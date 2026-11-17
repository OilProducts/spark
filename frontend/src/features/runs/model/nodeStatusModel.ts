import type { NodeStatus } from '@/store'
import type { VisitOutcome } from './visitModel'

/** The graph status a visit's outcome shows; an error the run continued past is still a failure. */
export const VISIT_NODE_STATUSES: Record<VisitOutcome, NodeStatus> = {
    completed: 'success',
    error: 'failed',
    failed: 'failed',
    interrupted: 'interrupted',
    running: 'running',
    waiting: 'waiting',
}

interface BuildRunNodeStatusesArgs {
    completedNodes: string[]
    nodeOutcomes: Record<string, string>
    currentNodeId: string | null
    liveNodeStatuses: Record<string, NodeStatus>
    gateNodeId: string | null
    isRunActive: boolean
    runStatus: string | null
    /** Each node's status as its latest visit in the run's Steps says, branches included. */
    visitStatuses?: Record<string, NodeStatus>
}

const TERMINAL_NODE_STATUSES = new Set<NodeStatus>(['success', 'failed'])

// Merges the durable checkpoint snapshot with live journal-driven statuses into
// the per-node status map the run graph renders. Precedence, lowest to highest:
// checkpoint completions, per-node checkpoint statuses (an errored node sits in
// completed_nodes too, so the error must override), live stage statuses, the
// active current node, the steps the run's visits say were interrupted, the failed run's
// terminal node, and a pending gate.
export function buildRunNodeStatuses({
    completedNodes,
    nodeOutcomes,
    currentNodeId,
    liveNodeStatuses,
    gateNodeId,
    isRunActive,
    runStatus,
    visitStatuses = {},
}: BuildRunNodeStatusesArgs): Record<string, NodeStatus> {
    const statuses: Record<string, NodeStatus> = {}
    for (const nodeId of completedNodes) {
        statuses[nodeId] = 'success'
    }
    for (const [nodeId, outcome] of Object.entries(nodeOutcomes)) {
        if (outcome === 'error' && statuses[nodeId]) {
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
    Object.assign(statuses, visitStatuses)
    if (runStatus === 'failed' && currentNodeId) {
        statuses[currentNodeId] = 'failed'
    }
    if (gateNodeId) {
        statuses[gateNodeId] = 'waiting'
    }
    return statuses
}
