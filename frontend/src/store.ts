import { create } from 'zustand'

export type ViewMode = 'editor' | 'execution'
export type NodeStatus = 'idle' | 'running' | 'success' | 'failed' | 'waiting'

export interface HumanGateOption {
    label: string
    value: string
}

export interface HumanGateState {
    id: string
    nodeId: string
    prompt: string
    options: HumanGateOption[]
    flowName?: string
}

export interface LogEntry {
    time: string
    msg: string
    type: 'info' | 'success' | 'error'
}

interface AppState {
    viewMode: ViewMode
    setViewMode: (mode: ViewMode) => void
    activeFlow: string | null
    setActiveFlow: (flow: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void

    logs: LogEntry[]
    addLog: (entry: LogEntry) => void
    clearLogs: () => void

    nodeStatuses: Record<string, NodeStatus>
    setNodeStatus: (nodeId: string, status: NodeStatus) => void
    resetNodeStatuses: () => void

    humanGate: HumanGateState | null
    setHumanGate: (gate: HumanGateState | null) => void
    clearHumanGate: () => void
}

export const useStore = create<AppState>((set) => ({
    viewMode: 'editor',
    setViewMode: (mode) => set({ viewMode: mode }),
    activeFlow: null,
    setActiveFlow: (flow) => set({ activeFlow: flow }),
    selectedNodeId: null,
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),

    logs: [],
    addLog: (entry) => set((state) => ({ logs: [...state.logs, entry] })),
    clearLogs: () => set({ logs: [] }),

    nodeStatuses: {},
    setNodeStatus: (nodeId, status) =>
        set((state) => ({ nodeStatuses: { ...state.nodeStatuses, [nodeId]: status } })),
    resetNodeStatuses: () => set({ nodeStatuses: {} }),

    humanGate: null,
    setHumanGate: (gate) => set({ humanGate: gate }),
    clearHumanGate: () => set({ humanGate: null }),
}))
