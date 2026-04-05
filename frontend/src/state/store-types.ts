import type { RunRecord } from '@/features/runs/model/shared'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import type {
    ExecutionSessionSlice,
    HomeSessionSlice,
    ResourceStatus,
    RunsSessionSlice,
    TriggersSessionSlice,
} from './viewSessionTypes'

export type ViewMode = 'home' | 'projects' | 'editor' | 'execution' | 'triggers' | 'settings' | 'runs'
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
export type PlanStatus = 'draft' | 'approved' | 'rejected' | 'revision-requested'
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

export interface GraphAttrs {
    'spark.title'?: string
    'spark.description'?: string
    'spark.launch_inputs'?: string
    goal?: string
    label?: string
    model_stylesheet?: string
    default_max_retries?: number | string
    default_max_retry?: number | string
    retry_target?: string
    fallback_retry_target?: string
    default_fidelity?: string
    'stack.child_dotfile'?: string
    'stack.child_workdir'?: string
    'tool.hooks.pre'?: string
    'tool.hooks.post'?: string
    ui_default_llm_model?: string
    ui_default_llm_provider?: string
    ui_default_reasoning_effort?: string
}

export type GraphAttrErrors = Partial<Record<keyof GraphAttrs, string>>

export interface RegisteredProject {
    directoryPath: string
    isFavorite: boolean
    lastAccessedAt: string | null
}

export interface ProjectRegistrationResult {
    ok: boolean
    normalizedPath?: string
    error?: string
}

export interface ProjectEventLogEntry {
    message: string
    timestamp: string
}

export interface ArtifactProvenanceReference {
    source: string
    referenceId: string
    capturedAt: string
    runId?: string | null
    gitBranch?: string | null
    gitCommit?: string | null
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
    reasoning_effort: string
}

export type ExecutionContinuationFlowSourceMode = 'snapshot' | 'flow_name'

export interface ExecutionContinuationDraft {
    sourceRunId: string
    sourceFlowName: string | null
    sourceWorkingDirectory: string
    sourceModel: string | null
    flowSourceMode: ExecutionContinuationFlowSourceMode
    startNodeId: string | null
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

export interface EditorViewSession {
    flowName: string | null
    selectedNodeId: string | null
    selectedEdgeId: string | null
    viewport: CanvasViewportState | null
    sidebarWidth: number
    graphAttrs: GraphAttrs
    diagnostics: DiagnosticEntry[]
    hasValidationErrors: boolean
    rawDotDraft: string
    rawHandoffError: string | null
    rawMode: 'structured' | 'raw'
    saveState: SaveState
}

export interface ExecutionViewSession {
    flowName: string | null
    selectedRunId: string | null
    selectedNodeId: string | null
    selectedEdgeId: string | null
    viewport: CanvasViewportState | null
    graphAttrs: GraphAttrs
    diagnostics: DiagnosticEntry[]
    hasValidationErrors: boolean
    launchInputValues: Record<string, string>
    launchError: string | null
}

export interface ProjectSessionState {
    workingDir: string
    conversationId: string | null
    projectEventLog: ProjectEventLogEntry[]
    specId: string | null
    specStatus: 'draft' | 'approved'
    specProvenance?: ArtifactProvenanceReference | null
    planId: string | null
    planStatus: PlanStatus
    planProvenance?: ArtifactProvenanceReference | null
}

export type ProjectSessionStatePatch = Partial<ProjectSessionState>

export interface ProjectSessionArtifactState {
    conversationId: string | null
    specId: string | null
    specStatus: 'draft' | 'approved'
    planId: string | null
    planStatus: PlanStatus
}

export interface HydratedProjectRecord {
    directoryPath: string
    isFavorite: boolean
    lastAccessedAt: string | null
    activeConversationId?: string | null
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
    appendProjectEventEntry: (entry: ProjectEventLogEntry) => void
    updateProjectSessionState: (projectPath: string, patch: ProjectSessionStatePatch) => void
    setSpecId: (id: string | null) => void
    setSpecStatus: (status: 'draft' | 'approved') => void
    setSpecProvenance: (provenance: ArtifactProvenanceReference | null) => void
    setPlanId: (id: string | null) => void
    setPlanStatus: (status: PlanStatus) => void
    setPlanProvenance: (provenance: ArtifactProvenanceReference | null) => void
    getProjectSessionArtifactState: (projectPath: string | null) => ProjectSessionArtifactState | null
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
    runRecordOverrides: Record<string, Partial<RunRecord>>
    setRunRecordOverride: (runId: string, patch: Partial<RunRecord> | null) => void
    clearRunRecordOverrides: () => void
    runGraphAttrs: GraphAttrs
    replaceRunGraphAttrs: (attrs: GraphAttrs) => void
    runDiagnostics: DiagnosticEntry[]
    setRunDiagnostics: (diagnostics: DiagnosticEntry[]) => void
    clearRunDiagnostics: () => void
    runNodeDiagnostics: Record<string, DiagnosticEntry[]>
    runEdgeDiagnostics: Record<string, DiagnosticEntry[]>
    runHasValidationErrors: boolean
    logs: LogEntry[]
    addLog: (entry: LogEntry) => void
    clearLogs: () => void
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

export interface ExecutionLaunchSlice {
    executionFlow: string | null
    setExecutionFlow: (flow: string | null) => void
    executionContinuation: ExecutionContinuationDraft | null
    setExecutionContinuation: (draft: ExecutionContinuationDraft | null) => void
    clearExecutionContinuation: () => void
    setExecutionContinuationFlowSourceMode: (mode: ExecutionContinuationFlowSourceMode) => void
    setExecutionContinuationStartNode: (nodeId: string | null) => void
    executionGraphAttrs: GraphAttrs
    replaceExecutionGraphAttrs: (attrs: GraphAttrs) => void
    executionDiagnostics: DiagnosticEntry[]
    setExecutionDiagnostics: (diagnostics: DiagnosticEntry[]) => void
    clearExecutionDiagnostics: () => void
    executionNodeDiagnostics: Record<string, DiagnosticEntry[]>
    executionEdgeDiagnostics: Record<string, DiagnosticEntry[]>
    executionHasValidationErrors: boolean
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
    rawDotDraft: string
    setRawDotDraft: (value: string) => void
    rawHandoffError: string | null
    setRawHandoffError: (value: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void
    selectedEdgeId: string | null
    setSelectedEdgeId: (id: string | null) => void
    workingDir: string
    setWorkingDir: (value: string) => void
    model: string
    setModel: (value: string) => void
    graphAttrs: GraphAttrs
    graphAttrErrors: GraphAttrErrors
    graphAttrsUserEditVersion: number
    setGraphAttrs: (attrs: GraphAttrs) => void
    replaceGraphAttrs: (attrs: GraphAttrs) => void
    updateGraphAttr: (key: keyof GraphAttrs, value: string) => void
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
    & ExecutionLaunchSlice
    & ExecutionSessionSlice
    & RunInspectorSlice
    & RunsSessionSlice
    & TriggersSessionSlice
    & HomeSessionSlice
    & EditorSlice
