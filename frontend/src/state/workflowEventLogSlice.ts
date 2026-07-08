import { type StateCreator } from 'zustand'
import type { AppState } from './store-types'

export interface WorkflowLogEntry {
    id: string
    seq: number
    timestamp: string
    kind: string
    message: string
    project_path: string
    run_id: string
    flow_name: string
    node_id?: string | null
}

export interface WorkflowEventLogSlice {
    workflowEventLog: WorkflowLogEntry[]
    applyWorkflowLogEntries: (entries: WorkflowLogEntry[]) => void
}

const WORKFLOW_EVENT_LOG_CAP = 200

export const createWorkflowEventLogSlice: StateCreator<AppState, [], [], WorkflowEventLogSlice> = (set) => ({
    workflowEventLog: [],
    applyWorkflowLogEntries: (entries) =>
        set((state) => {
            if (entries.length === 0) {
                return {}
            }
            const byId = new Map(state.workflowEventLog.map((entry) => [entry.id, entry]))
            for (const entry of entries) {
                byId.set(entry.id, entry)
            }
            const merged = Array.from(byId.values()).sort((left, right) => left.seq - right.seq)
            return {
                workflowEventLog: merged.slice(Math.max(0, merged.length - WORKFLOW_EVENT_LOG_CAP)),
            }
        }),
})
