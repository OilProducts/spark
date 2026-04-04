import { type StateCreator } from 'zustand'
import { buildDiagnosticMaps, normalizeGraphAttrs } from './store-helpers'
import type { AppState, RunInspectorSlice } from './store-types'

export const createRunInspectorSlice: StateCreator<AppState, [], [], RunInspectorSlice> = (set) => ({
    selectedRunId: null,
    setSelectedRunId: (id) => set({ selectedRunId: id }),
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    setSelectedRunSnapshot: ({ record, completedNodes = [], fetchedAtMs = null }) =>
        set({
            selectedRunRecord: record,
            selectedRunCompletedNodes: completedNodes,
            selectedRunStatusFetchedAtMs: fetchedAtMs,
        }),
    setSelectedRunStatusSync: (status, error = null) =>
        set({
            selectedRunStatusSync: status,
            selectedRunStatusError: error,
        }),
    runRecordOverrides: {},
    setRunRecordOverride: (runId, patch) =>
        set((state) => {
            const nextOverrides = { ...state.runRecordOverrides }
            if (!patch) {
                delete nextOverrides[runId]
            } else {
                nextOverrides[runId] = {
                    ...(state.runRecordOverrides[runId] ?? {}),
                    ...patch,
                }
            }
            return {
                runRecordOverrides: nextOverrides,
            }
        }),
    clearRunRecordOverrides: () => set({ runRecordOverrides: {} }),
    runGraphAttrs: {},
    replaceRunGraphAttrs: (attrs) =>
        set({
            runGraphAttrs: normalizeGraphAttrs(attrs),
        }),
    runDiagnostics: [],
    setRunDiagnostics: (diagnostics) =>
        set(() => {
            const { nodeDiagnostics, edgeDiagnostics } = buildDiagnosticMaps(diagnostics)
            return {
                runDiagnostics: diagnostics,
                runNodeDiagnostics: nodeDiagnostics,
                runEdgeDiagnostics: edgeDiagnostics,
                runHasValidationErrors: diagnostics.some((diag) => diag.severity === 'error'),
            }
        }),
    clearRunDiagnostics: () =>
        set({
            runDiagnostics: [],
            runNodeDiagnostics: {},
            runEdgeDiagnostics: {},
            runHasValidationErrors: false,
        }),
    runNodeDiagnostics: {},
    runEdgeDiagnostics: {},
    runHasValidationErrors: false,
    logs: [],
    addLog: (entry) => set((state) => ({ logs: [...state.logs, entry] })),
    clearLogs: () => set({ logs: [] }),
    runtimeStatus: 'idle',
    setRuntimeStatus: (status) => set({ runtimeStatus: status }),
    runtimeOutcome: null,
    runtimeOutcomeReasonCode: null,
    runtimeOutcomeReasonMessage: null,
    setRuntimeOutcome: (outcome, outcomeReasonCode = null, outcomeReasonMessage = null) =>
        set({
            runtimeOutcome: outcome,
            runtimeOutcomeReasonCode: outcomeReasonCode,
            runtimeOutcomeReasonMessage: outcomeReasonMessage,
        }),
    nodeStatuses: {},
    setNodeStatus: (nodeId, status) =>
        set((state) => ({ nodeStatuses: { ...state.nodeStatuses, [nodeId]: status } })),
    resetNodeStatuses: () => set({ nodeStatuses: {} }),
    humanGate: null,
    setHumanGate: (gate) => set({ humanGate: gate }),
    clearHumanGate: () => set({ humanGate: null }),
})
