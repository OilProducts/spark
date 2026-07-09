import type { RunRecord } from '@/features/runs/model/shared'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import type {
    HomeSessionSlice,
    ResourceStatus,
    RunsSessionSlice,
    TriggersSessionSlice,
} from './viewSessionTypes'
import type { WorkflowEventLogSlice } from './workflowEventLogSlice'

export type ViewMode = 'home' | 'projects' | 'editor' | 'triggers' | 'settings' | 'runs'
export type EditorMode = 'structured' | 'raw'
export type NodeStatus = 'idle' | 'running' | 'success' | 'failed' | 'waiting'
export type DiagnosticSeverity = 'error' | 'warning' | 'info'
export type RunOutcome = 'success' | 'failure'
export type RuntimeStatus =
    | 'idle'
    | 'running'
    | 'abort_requested'
    | 'cancel_requested'
    | 'aborted'
    | 'canceled'
    | 'failed'
    | 'validation_error'
    | 'completed'
export type SaveState = 'idle' | 'saving' | 'saved' | 'error' | 'conflict'
export type SaveErrorKind = 'parse_error' | 'validation_error' | 'conflict' | 'network' | 'http' | 'unknown'
export type SelectedRunStatusSync = 'idle' | 'loading' | 'ready' | 'degraded'
export type { ResourceStatus }

export interface HumanGateOption {
    label: string
    value: string
}

export interface HumanGateState {
    id: string
    runId: string
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

export interface FlowDefinitionMetadata {
    [key: string]: unknown
    schema_version?: string
    id?: string
    title?: string
    description?: string
    inputs?: string
    result_node?: string
    result_summary_enabled?: string
    result_summary_prompt?: string
    goal?: string
    max_retries?: number | string
    fidelity?: string
    llm_model?: string
    llm_provider?: string
    llm_profile?: string
    reasoning_effort?: string
}

export type GraphAttrs = FlowDefinitionMetadata
export type FlowMetadataErrors = Partial<Record<keyof FlowDefinitionMetadata, string>>
export type GraphAttrErrors = FlowMetadataErrors

export interface RegisteredProject {
    directoryPath: string
    isFavorite: boolean
    lastAccessedAt: string | null
    executionProfileId?: string | null
}

export interface ProjectRegistrationResult {
    ok: boolean
    normalizedPath?: string
    error?: string
}

export interface DiagnosticEntry {
    rule_id: string
    severity: DiagnosticSeverity
    message: string
    line?: number
    node_id?: string | null
    edge?: [string, string] | null
    fix?: string | null
}

export interface UiDefaults {
    llm_model: string
    llm_provider: string
    llm_profile: string
    reasoning_effort: string
}

export interface RouteState {
    viewMode: ViewMode
    activeProjectPath: string | null
}

export interface CanvasViewportState {
    x: number
    y: number
    zoom: number
}

export interface ProjectSessionState {
    workingDir: string
    conversationId: string | null
}

export type ProjectSessionStatePatch = Partial<ProjectSessionState>

export interface HydratedProjectRecord {
    directoryPath: string
    isFavorite: boolean
    lastAccessedAt: string | null
    activeConversationId?: string | null
    executionProfileId?: string | null
}

export interface WorkspaceSlice {
    viewMode: ViewMode
    setViewMode: (mode: ViewMode) => void
    activeProjectPath: string | null
    setActiveProjectPath: (projectPath: string | null) => void
    projectRegistry: Record<string, RegisteredProject>
    hydrateProjectRegistry: (projects: HydratedProjectRecord[]) => void
    upsertProjectRegistryEntry: (project: HydratedProjectRecord) => void
    removeProject: (directoryPath: string, nextActiveProjectPath?: string | null) => void
    recentProjectPaths: string[]
    projectSessionsByPath: Record<string, ProjectSessionState>
    projectRegistrationError: string | null
    registerProject: (directoryPath: string) => ProjectRegistrationResult
    updateProjectPath: (currentDirectoryPath: string, nextDirectoryPath: string) => ProjectRegistrationResult
    toggleProjectFavorite: (projectPath: string) => void
    setProjectRegistrationError: (error: string | null) => void
    clearProjectRegistrationError: () => void
    activeFlow: string | null
    setActiveFlow: (flow: string | null) => void
    setConversationId: (id: string | null) => void
    updateProjectSessionState: (projectPath: string, patch: ProjectSessionStatePatch) => void
}

export interface RunInspectorSlice {
    selectedRunId: string | null
    setSelectedRunId: (id: string | null) => void
    selectedRunRecord: RunRecord | null
    selectedRunCompletedNodes: string[]
    selectedRunStatusSync: SelectedRunStatusSync
    selectedRunStatusError: string | null
    selectedRunStatusFetchedAtMs: number | null
    setSelectedRunSnapshot: (snapshot: {
        record: RunRecord | null
        completedNodes?: string[]
        fetchedAtMs?: number | null
    }) => void
    setSelectedRunStatusSync: (status: SelectedRunStatusSync, error?: string | null) => void
    runGraphAttrs: FlowDefinitionMetadata
    replaceRunGraphAttrs: (attrs: FlowDefinitionMetadata) => void
    runDiagnostics: DiagnosticEntry[]
    setRunDiagnostics: (diagnostics: DiagnosticEntry[]) => void
    clearRunDiagnostics: () => void
    runNodeDiagnostics: Record<string, DiagnosticEntry[]>
    runEdgeDiagnostics: Record<string, DiagnosticEntry[]>
    runHasValidationErrors: boolean
    runtimeStatus: RuntimeStatus
    setRuntimeStatus: (status: RuntimeStatus) => void
    runtimeOutcome: RunOutcome | null
    runtimeOutcomeReasonCode: string | null
    runtimeOutcomeReasonMessage: string | null
    setRuntimeOutcome: (
        outcome: RunOutcome | null,
        outcomeReasonCode?: string | null,
        outcomeReasonMessage?: string | null,
    ) => void
    nodeStatuses: Record<string, NodeStatus>
    setNodeStatus: (nodeId: string, status: NodeStatus) => void
    resetNodeStatuses: () => void
    humanGate: HumanGateState | null
    setHumanGate: (gate: HumanGateState | null) => void
    clearHumanGate: () => void
}

export interface EditorNodeInspectorSessionState {
    showAdvanced: boolean
    readsContextDraft: string
    readsContextError: string | null
    writesContextDraft: string
    writesContextError: string | null
}

export interface EditorSlice {
    editorSidebarWidth: number
    setEditorSidebarWidth: (width: number) => void
    editorMode: EditorMode
    setEditorMode: (mode: EditorMode) => void
    rawYamlDraft: string
    setRawYamlDraft: (value: string) => void
    rawHandoffError: string | null
    setRawHandoffError: (value: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void
    selectedEdgeId: string | null
    setSelectedEdgeId: (id: string | null) => void
    pendingEditorNodeSelection: { flowName: string; nodeId: string | null } | null
    setPendingEditorNodeSelection: (selection: { flowName: string; nodeId: string | null } | null) => void
    workingDir: string
    setWorkingDir: (value: string) => void
    model: string
    setModel: (value: string) => void
    flowMetadata: FlowDefinitionMetadata
    flowMetadataErrors: FlowMetadataErrors
    flowMetadataUserEditVersion: number
    setFlowMetadata: (metadata: FlowDefinitionMetadata) => void
    replaceFlowMetadata: (metadata: FlowDefinitionMetadata) => void
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
    graphAttrs: FlowDefinitionMetadata
    graphAttrErrors: FlowMetadataErrors
    graphAttrsUserEditVersion: number
    setGraphAttrs: (attrs: FlowDefinitionMetadata) => void
    replaceGraphAttrs: (attrs: FlowDefinitionMetadata) => void
    updateGraphAttr: (key: keyof FlowDefinitionMetadata, value: string) => void
    diagnostics: DiagnosticEntry[]
    setDiagnostics: (diagnostics: DiagnosticEntry[]) => void
    clearDiagnostics: () => void
    nodeDiagnostics: Record<string, DiagnosticEntry[]>
    edgeDiagnostics: Record<string, DiagnosticEntry[]>
    hasValidationErrors: boolean
    suppressPreview: boolean
    setSuppressPreview: (value: boolean) => void
    uiDefaults: UiDefaults
    setUiDefaults: (values: Partial<UiDefaults>) => void
    setUiDefault: (key: keyof UiDefaults, value: string) => void
    saveState: SaveState
    saveStateVersion: number
    saveErrorMessage: string | null
    saveErrorKind: SaveErrorKind | null
    editorGraphSettingsPanelOpenByFlow: Record<string, boolean>
    setEditorGraphSettingsPanelOpen: (flowName: string, isOpen: boolean) => void
    editorShowAdvancedFlowMetadataByFlow: Record<string, boolean>
    setEditorShowAdvancedFlowMetadata: (flowName: string, showAdvanced: boolean) => void
    editorExpandChildFlowsByFlow: Record<string, boolean>
    setEditorExpandChildFlows: (flowName: string, expandChildren: boolean) => void
    editorShowAdvancedGraphAttrsByFlow: Record<string, boolean>
    setEditorShowAdvancedGraphAttrs: (flowName: string, showAdvanced: boolean) => void
    editorLaunchInputDraftsByFlow: Record<string, LaunchInputDefinition[]>
    editorLaunchInputDraftErrorByFlow: Record<string, string | null>
    setEditorLaunchInputDraftState: (
        flowName: string,
        drafts: LaunchInputDefinition[],
        error: string | null,
    ) => void
    editorNodeInspectorSessionsByNodeId: Record<string, EditorNodeInspectorSessionState>
    updateEditorNodeInspectorSession: (nodeId: string, patch: Partial<EditorNodeInspectorSessionState>) => void
    markSaveInFlight: () => void
    markSaveSuccess: () => void
    markSaveConflict: (message: string) => void
    markSaveFailure: (message: string, kind?: SaveErrorKind) => void
    resetSaveState: () => void
}

export type AppState =
    & WorkspaceSlice
    & WorkflowEventLogSlice
    & RunInspectorSlice
    & RunsSessionSlice
    & TriggersSessionSlice
    & HomeSessionSlice
    & EditorSlice
