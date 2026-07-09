import { type StateCreator } from 'zustand'
import {
    buildDiagnosticMaps,
    DEFAULT_WORKING_DIRECTORY,
    deriveGraphAttrErrors,
    loadUiDefaults,
    normalizeGraphAttrs,
    normalizeGraphAttrValue,
    resolveProjectSessionState,
    saveUiDefaults,
    validateGraphAttrValue,
} from './store-helpers'
import type { AppState, EditorSlice } from './store-types'
import { initialWorkspaceEditorState } from './workspaceSlice'

const DEFAULT_EDITOR_SIDEBAR_WIDTH = 288
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

export const createEditorSlice: StateCreator<AppState, [], [], EditorSlice> = (set) => ({
    editorSidebarWidth: DEFAULT_EDITOR_SIDEBAR_WIDTH,
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
    editorMode: 'structured',
    setEditorMode: (mode) => set({ editorMode: mode }),
    rawYamlDraft: '',
    setRawYamlDraft: (value) => set({ rawYamlDraft: value }),
    rawHandoffError: null,
    setRawHandoffError: (value) => set({ rawHandoffError: value }),
    selectedNodeId: null,
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),
    selectedEdgeId: null,
    setSelectedEdgeId: (id) => set({ selectedEdgeId: id }),
    pendingEditorNodeSelection: null,
    setPendingEditorNodeSelection: (selection) => set({ pendingEditorNodeSelection: selection }),
    workingDir: initialWorkspaceEditorState.workingDir || DEFAULT_WORKING_DIRECTORY,
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
    editorGraphSettingsPanelOpenByFlow: {},
    setEditorGraphSettingsPanelOpen: (flowName, isOpen) =>
        set((state) => ({
            editorGraphSettingsPanelOpenByFlow: {
                ...state.editorGraphSettingsPanelOpenByFlow,
                [flowName]: isOpen,
            },
        })),
    editorExpandChildFlowsByFlow: {},
    setEditorExpandChildFlows: (flowName, expandChildren) =>
        set((state) => ({
            editorExpandChildFlowsByFlow: {
                ...state.editorExpandChildFlowsByFlow,
                [flowName]: expandChildren,
            },
        })),
    editorShowAdvancedFlowMetadataByFlow: {},
    setEditorShowAdvancedFlowMetadata: (flowName, showAdvanced) =>
        set((state) => ({
            editorShowAdvancedFlowMetadataByFlow: {
                ...state.editorShowAdvancedFlowMetadataByFlow,
                [flowName]: showAdvanced,
            },
        })),
    editorShowAdvancedGraphAttrsByFlow: {},
    setEditorShowAdvancedGraphAttrs: (flowName, showAdvanced) =>
        set((state) => ({
            editorShowAdvancedGraphAttrsByFlow: {
                ...state.editorShowAdvancedGraphAttrsByFlow,
                [flowName]: showAdvanced,
            },
            editorShowAdvancedFlowMetadataByFlow: {
                ...state.editorShowAdvancedFlowMetadataByFlow,
                [flowName]: showAdvanced,
            },
        })),
    editorLaunchInputDraftsByFlow: {},
    editorLaunchInputDraftErrorByFlow: {},
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
    editorNodeInspectorSessionsByNodeId: {},
    updateEditorNodeInspectorSession: (nodeId, patch) =>
        set((state) => ({
            editorNodeInspectorSessionsByNodeId: {
                ...state.editorNodeInspectorSessionsByNodeId,
                [nodeId]: {
                    ...DEFAULT_EDITOR_NODE_INSPECTOR_SESSION,
                    ...(state.editorNodeInspectorSessionsByNodeId[nodeId] ?? {}),
                    ...patch,
                },
            },
        })),
    flowMetadata: {},
    flowMetadataErrors: {},
    flowMetadataUserEditVersion: 0,
    graphAttrs: {},
    graphAttrErrors: {},
    graphAttrsUserEditVersion: 0,
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
    diagnostics: [],
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
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    suppressPreview: false,
    setSuppressPreview: (value) => set({ suppressPreview: value }),
    uiDefaults: loadUiDefaults(),
    setUiDefaults: (values) =>
        set((state) => {
            const next = { ...state.uiDefaults, ...values }
            saveUiDefaults(next)
            return { uiDefaults: next }
        }),
    setUiDefault: (key, value) =>
        set((state) => {
            const next = { ...state.uiDefaults, [key]: value }
            saveUiDefaults(next)
            return { uiDefaults: next }
        }),
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    markSaveInFlight: () =>
        set((state) => ({
            saveState: 'saving',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: null,
            saveErrorKind: null,
        })),
    markSaveSuccess: () =>
        set((state) => ({
            saveState: 'saved',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: null,
            saveErrorKind: null,
        })),
    markSaveConflict: (message) =>
        set((state) => ({
            saveState: 'conflict',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: message || 'Flow save conflict detected.',
            saveErrorKind: 'conflict',
        })),
    markSaveFailure: (message, kind = 'unknown') =>
        set((state) => ({
            saveState: 'error',
            saveStateVersion: state.saveStateVersion + 1,
            saveErrorMessage: message || 'Flow save failed.',
            saveErrorKind: kind,
        })),
    resetSaveState: () =>
        set({
            saveState: 'idle',
            saveErrorMessage: null,
            saveErrorKind: null,
        }),
})
