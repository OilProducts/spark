import { type StateCreator } from 'zustand'
import { buildRunsScopeKey } from './runsSessionScope'
import { buildDiagnosticMaps, normalizeGraphAttrs } from './store-helpers'
import type { AppState, RunInspectorSlice } from './store-types'

export const createRunInspectorSlice: StateCreator<AppState, [], [], RunInspectorSlice> = (set) => ({
    selectedRunId: null,
    setSelectedRunId: (id) =>
        set((state) => {
            const scopeKey = buildRunsScopeKey(state.runsListSession.scopeMode, state.activeProjectPath)
            return {
                selectedRunId: id,
                runsListSession: {
                    ...state.runsListSession,
                    selectedRunIdByScopeKey: {
                        ...state.runsListSession.selectedRunIdByScopeKey,
                        [scopeKey]: id,
                    },
                },
            }
        }),
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
