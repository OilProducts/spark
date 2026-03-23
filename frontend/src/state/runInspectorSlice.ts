import { type StateCreator } from 'zustand'
import { buildDiagnosticMaps, normalizeGraphAttrs } from './store-helpers'
import type { AppState, RunInspectorSlice } from './store-types'

export const createRunInspectorSlice: StateCreator<AppState, [], [], RunInspectorSlice> = (set) => ({
    executionFlow: null,
    setExecutionFlow: (flow) =>
        set({
            executionFlow: flow,
            executionGraphAttrs: {},
            executionDiagnostics: [],
            executionNodeDiagnostics: {},
            executionEdgeDiagnostics: {},
            executionHasValidationErrors: false,
        }),
    selectedRunId: null,
    setSelectedRunId: (id) => set({ selectedRunId: id }),
    executionGraphAttrs: {},
    replaceExecutionGraphAttrs: (attrs) =>
        set({
            executionGraphAttrs: normalizeGraphAttrs(attrs),
        }),
    executionDiagnostics: [],
    setExecutionDiagnostics: (diagnostics) =>
        set(() => {
            const { nodeDiagnostics, edgeDiagnostics } = buildDiagnosticMaps(diagnostics)
            return {
                executionDiagnostics: diagnostics,
                executionNodeDiagnostics: nodeDiagnostics,
                executionEdgeDiagnostics: edgeDiagnostics,
                executionHasValidationErrors: diagnostics.some((diag) => diag.severity === 'error'),
            }
        }),
    clearExecutionDiagnostics: () =>
        set({
            executionDiagnostics: [],
            executionNodeDiagnostics: {},
            executionEdgeDiagnostics: {},
            executionHasValidationErrors: false,
        }),
    executionNodeDiagnostics: {},
    executionEdgeDiagnostics: {},
    executionHasValidationErrors: false,
    logs: [],
    addLog: (entry) => set((state) => ({ logs: [...state.logs, entry] })),
    clearLogs: () => set({ logs: [] }),
    runtimeStatus: 'idle',
    setRuntimeStatus: (status) => set({ runtimeStatus: status }),
    nodeStatuses: {},
    setNodeStatus: (nodeId, status) =>
        set((state) => ({ nodeStatuses: { ...state.nodeStatuses, [nodeId]: status } })),
    resetNodeStatuses: () => set({ nodeStatuses: {} }),
    humanGate: null,
    setHumanGate: (gate) => set({ humanGate: gate }),
    clearHumanGate: () => set({ humanGate: null }),
})
