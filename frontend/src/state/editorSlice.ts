import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { type StateCreator } from 'zustand'
import {
    buildDiagnosticMaps,
    DEFAULT_ROUTE_STATE,
    DEFAULT_WORKING_DIRECTORY,
    deriveGraphAttrErrors,
    DEFAULT_UI_DEFAULTS,
    loadRouteState,
    normalizeGraphAttrs,
    normalizeGraphAttrValue,
    resolveProjectSessionState,
    validateGraphAttrValue,
} from './store-helpers'
import type { AppState, EditorSlice, RouteState } from './store-types'

const DEFAULT_EDITOR_SIDEBAR_WIDTH = 336
const MIN_EDITOR_SIDEBAR_WIDTH = 256
const MAX_EDITOR_SIDEBAR_WIDTH = 560
const DEFAULT_EDITOR_NODE_INSPECTOR_SESSION = {
    showAdvanced: false,
    readsContextDraft: '',
    readsContextError: null,
    writesContextDraft: '',
    writesContextError: null,
}

const flowMetadataEqual = (left: Record<string, unknown>, right: Record<string, unknown>) => {
    const leftKeys = Object.keys(left)
    const rightKeys = Object.keys(right)
    if (leftKeys.length !== rightKeys.length) {
        return false
    }
    return leftKeys.every((key) => left[key] === right[key])
}

const deriveNextFlowMetadataState = (
    state: Pick<EditorSlice, 'flowMetadata' | 'flowMetadataErrors' | 'flowMetadataUserEditVersion'>,
    metadata: EditorSlice['flowMetadata'],
    markDirty: boolean,
) => {
    const normalizedMetadata = normalizeGraphAttrs(metadata)
    const nextFlowMetadataErrors = deriveGraphAttrErrors(normalizedMetadata)
    const metadataUnchanged = flowMetadataEqual(
        state.flowMetadata as Record<string, unknown>,
        normalizedMetadata as Record<string, unknown>,
    )
    const errorsUnchanged = flowMetadataEqual(
        state.flowMetadataErrors as Record<string, unknown>,
        nextFlowMetadataErrors as Record<string, unknown>,
    )
    if (metadataUnchanged && errorsUnchanged) {
        return state
    }
    return {
        flowMetadata: normalizedMetadata,
        flowMetadataErrors: nextFlowMetadataErrors,
        flowMetadataUserEditVersion: markDirty
            ? state.flowMetadataUserEditVersion + 1
            : state.flowMetadataUserEditVersion,
        graphAttrs: normalizedMetadata,
        graphAttrErrors: nextFlowMetadataErrors,
        graphAttrsUserEditVersion: markDirty
            ? state.flowMetadataUserEditVersion + 1
            : state.flowMetadataUserEditVersion,
    }
}

/**
 * The editor's state in a fresh environment, open on `route`'s project: what that environment's
 * flows, stored layouts and preferences put here, at their defaults until its preferences load.
 */
export const initialEditorSessionState = (route: RouteState = DEFAULT_ROUTE_STATE) => ({
    clientPreferencesLoaded: false,
    clientFlowNodePositions: {},
    clientFlowEdgePorts: {},
    chatProjectOrder: [],
    clientRunPresentation: {},
    editorSidebarWidth: DEFAULT_EDITOR_SIDEBAR_WIDTH,
    preferredEditorMode: 'structured',
    preferredShowThinking: true,
    preferredAdvancedControls: false,
    preferredExpandChildFlows: false,
    preferredGraphSettingsOpen: false,
    preferredHomeSidebarPrimarySplitRatio: null,
    editorMode: 'structured',
    selectedNodeId: null,
    selectedEdgeId: null,
    pendingEditorNodeSelection: null,
    workingDir: route.activeProjectPath || DEFAULT_WORKING_DIRECTORY,
    editorGraphSettingsPanelOpenByFlow: {},
    editorExpandChildFlowsByFlow: {},
    editorShowAdvancedFlowMetadataByFlow: {},
    editorShowAdvancedGraphAttrsByFlow: {},
    editorLaunchInputDraftsByFlow: {},
    editorLaunchInputDraftErrorByFlow: {},
    editorNodeInspectorSessionsByNodeId: {},
    flowMetadata: {},
    flowMetadataErrors: {},
    flowMetadataUserEditVersion: 0,
    graphAttrs: {},
    graphAttrErrors: {},
    graphAttrsUserEditVersion: 0,
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    uiDefaults: { ...DEFAULT_UI_DEFAULTS },
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    saveErrorProblems: null,
}) satisfies Partial<EditorSlice>

export const createEditorSlice: StateCreator<AppState, [], [], EditorSlice> = (set, get) => ({
    ...initialEditorSessionState(loadRouteState()),
    setEditorSidebarWidth: (width) =>
        set((state) => {
            const nextWidth = Math.min(
                Math.max(Math.round(width), MIN_EDITOR_SIDEBAR_WIDTH),
                MAX_EDITOR_SIDEBAR_WIDTH,
            )
            if (nextWidth === state.editorSidebarWidth) {
                return state
            }
            return {
                editorSidebarWidth: nextWidth,
            }
        }),
    setClientRunPresentation: (patch) => {
        const run_presentation = { ...get().clientRunPresentation, ...patch }
        set({ clientRunPresentation: run_presentation })
        completePreferenceInteraction({ run_presentation })
    },
    setPreferredShowThinking: (show) => {
        set({ preferredShowThinking: show })
        completePreferenceInteraction({ show_thinking: show })
    },
    setEditorMode: (mode) => set({ editorMode: mode }),
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),
    setSelectedEdgeId: (id) => set({ selectedEdgeId: id }),
    setPendingEditorNodeSelection: (selection) => set({ pendingEditorNodeSelection: selection }),
    setWorkingDir: (value) =>
        set((state) => {
            const nextProjectSessionStates = { ...state.projectSessionsByPath }
            if (state.activeProjectPath) {
                const scoped = resolveProjectSessionState(
                    nextProjectSessionStates[state.activeProjectPath],
                    state.activeProjectPath,
                )
                nextProjectSessionStates[state.activeProjectPath] = {
                    ...scoped,
                    workingDir: value,
                }
            }
            return {
                workingDir: value,
                projectSessionsByPath: nextProjectSessionStates,
            }
        }),
    model: '',
    setModel: (value) => set({ model: value }),
    setEditorGraphSettingsPanelOpen: (flowName, isOpen) => {
        completePreferenceInteraction({ graph_settings_open: isOpen })
        set((state) => ({
            preferredGraphSettingsOpen: isOpen,
            editorGraphSettingsPanelOpenByFlow: {
                ...state.editorGraphSettingsPanelOpenByFlow,
                [flowName]: isOpen,
            },
        }))
    },
    setEditorExpandChildFlows: (flowName, expandChildren) => {
        completePreferenceInteraction({ expand_child_flows: expandChildren })
        set((state) => ({
            preferredExpandChildFlows: expandChildren,
            editorExpandChildFlowsByFlow: {
                ...state.editorExpandChildFlowsByFlow,
                [flowName]: expandChildren,
            },
        }))
    },
    setEditorShowAdvancedFlowMetadata: (flowName, showAdvanced) => {
        completePreferenceInteraction({ show_advanced_controls: showAdvanced })
        set((state) => ({
            preferredAdvancedControls: showAdvanced,
            editorShowAdvancedFlowMetadataByFlow: {
                ...state.editorShowAdvancedFlowMetadataByFlow,
                [flowName]: showAdvanced,
            },
        }))
    },
    setEditorShowAdvancedGraphAttrs: (flowName, showAdvanced) => {
        completePreferenceInteraction({ show_advanced_controls: showAdvanced })
        set((state) => ({
            preferredAdvancedControls: showAdvanced,
            editorShowAdvancedGraphAttrsByFlow: {
                ...state.editorShowAdvancedGraphAttrsByFlow,
                [flowName]: showAdvanced,
            },
            editorShowAdvancedFlowMetadataByFlow: {
                ...state.editorShowAdvancedFlowMetadataByFlow,
                [flowName]: showAdvanced,
            },
        }))
    },
    setEditorLaunchInputDraftState: (flowName, drafts, error) =>
        set((state) => ({
            editorLaunchInputDraftsByFlow: {
                ...state.editorLaunchInputDraftsByFlow,
                [flowName]: drafts,
            },
            editorLaunchInputDraftErrorByFlow: {
                ...state.editorLaunchInputDraftErrorByFlow,
                [flowName]: error,
            },
        })),
    updateEditorNodeInspectorSession: (nodeId, patch) => {
        if (patch.showAdvanced !== undefined) completePreferenceInteraction({ show_advanced_controls: patch.showAdvanced })
        set((state) => ({
            preferredAdvancedControls: patch.showAdvanced ?? state.preferredAdvancedControls,
            editorNodeInspectorSessionsByNodeId: {
                ...state.editorNodeInspectorSessionsByNodeId,
                [nodeId]: {
                    ...(state.editorNodeInspectorSessionsByNodeId[nodeId] ?? {
                        ...DEFAULT_EDITOR_NODE_INSPECTOR_SESSION,
                        showAdvanced: state.preferredAdvancedControls,
                    }),
                    ...patch,
                },
            },
        }))
    },
    setFlowMetadata: (metadata) =>
        set((state) => deriveNextFlowMetadataState(state, metadata, true)),
    replaceFlowMetadata: (metadata) =>
        set((state) => deriveNextFlowMetadataState(state, metadata, false)),
    updateFlowMetadata: (key, value) =>
        set((state) => {
            const normalizedValue = normalizeGraphAttrValue(key, value)
            const currentValue = state.flowMetadata[key]
            const currentNormalizedValue = currentValue === undefined || currentValue === null
                ? ''
                : normalizeGraphAttrValue(key, String(currentValue))
            const currentError = state.flowMetadataErrors[key] ?? null
            const nextError = validateGraphAttrValue(key, normalizedValue)
            if (currentNormalizedValue === normalizedValue && currentError === nextError) {
                return state
            }
            return deriveNextFlowMetadataState(
                state,
                {
                    ...state.flowMetadata,
                    [key]: normalizedValue,
                },
                true,
            )
        }),
    setGraphAttrs: (attrs) =>
        set((state) => deriveNextFlowMetadataState(state, attrs, true)),
    replaceGraphAttrs: (attrs) =>
        set((state) => deriveNextFlowMetadataState(state, attrs, false)),
    updateGraphAttr: (key, value) =>
        set((state) => {
            const normalizedValue = normalizeGraphAttrValue(key, value)
            const currentValue = state.flowMetadata[key]
            const currentNormalizedValue = currentValue === undefined || currentValue === null
                ? ''
                : normalizeGraphAttrValue(key, String(currentValue))
            const currentError = state.flowMetadataErrors[key] ?? null
            const nextError = validateGraphAttrValue(key, normalizedValue)
            if (currentNormalizedValue === normalizedValue && currentError === nextError) {
                return state
            }
            return deriveNextFlowMetadataState(
                state,
                {
                    ...state.flowMetadata,
                    [key]: normalizedValue,
                },
                true,
            )
        }),
    setDiagnostics: (diagnostics) =>
        set(() => {
            const { nodeDiagnostics, edgeDiagnostics } = buildDiagnosticMaps(diagnostics)
            return {
                diagnostics,
                nodeDiagnostics,
                edgeDiagnostics,
                hasValidationErrors: diagnostics.some((diag) => diag.severity === 'error'),
            }
        }),
    clearDiagnostics: () =>
        set(() => ({
            diagnostics: [],
            nodeDiagnostics: {},
            edgeDiagnostics: {},
            hasValidationErrors: false,
        })),
    suppressPreview: false,
    setSuppressPreview: (value) => set({ suppressPreview: value }),
    setUiDefaults: (values) =>
        set((state) => {
            const next = { ...state.uiDefaults, ...values }
            return { uiDefaults: next }
        }),
    markSaveInFlight: () =>
        set((state) => ({
            saveState: 'saving',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: null,
            saveErrorKind: null,
            saveErrorProblems: null,
        })),
    markSaveSuccess: () =>
        set((state) => ({
            saveState: 'saved',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: null,
            saveErrorKind: null,
            saveErrorProblems: null,
        })),
    markSaveConflict: (message) =>
        set((state) => ({
            saveState: 'conflict',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: message || 'Flow save conflict detected.',
            saveErrorKind: 'conflict',
            saveErrorProblems: null,
        })),
    markSaveFailure: (message, kind = 'unknown', problems = null) =>
        set((state) => ({
            saveState: 'error',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: message || 'Flow save failed.',
            saveErrorKind: kind,
            saveErrorProblems: problems,
        })),
    resetSaveState: () =>
        set({
            saveState: 'idle',
            saveErrorMessage: null,
            saveErrorKind: null,
            saveErrorProblems: null,
        }),
})
