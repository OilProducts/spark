export type ViewMode = 'home' | 'projects' | 'editor' | 'execution' | 'triggers' | 'settings' | 'runs'
export type NodeStatus = 'idle' | 'running' | 'success' | 'failed' | 'waiting'
export type DiagnosticSeverity = 'error' | 'warning' | 'info'
export type RuntimeStatus =
    | 'idle'
    | 'running'
    | 'abort_requested'
    | 'cancel_requested'
    | 'aborted'
    | 'canceled'
    | 'failed'
    | 'validation_error'
    | 'success'
export type SaveState = 'idle' | 'saving' | 'saved' | 'error' | 'conflict'
export type SaveErrorKind = 'parse_error' | 'validation_error' | 'conflict' | 'network' | 'http' | 'unknown'
export type PlanStatus = 'draft' | 'approved' | 'rejected' | 'revision-requested'

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
    executionFlow: string | null
    setExecutionFlow: (flow: string | null) => void
    selectedRunId: string | null
    setSelectedRunId: (id: string | null) => void
    executionGraphAttrs: GraphAttrs
    replaceExecutionGraphAttrs: (attrs: GraphAttrs) => void
    executionDiagnostics: DiagnosticEntry[]
    setExecutionDiagnostics: (diagnostics: DiagnosticEntry[]) => void
    clearExecutionDiagnostics: () => void
    executionNodeDiagnostics: Record<string, DiagnosticEntry[]>
    executionEdgeDiagnostics: Record<string, DiagnosticEntry[]>
    executionHasValidationErrors: boolean
    logs: LogEntry[]
    addLog: (entry: LogEntry) => void
    clearLogs: () => void
    runtimeStatus: RuntimeStatus
    setRuntimeStatus: (status: RuntimeStatus) => void
    nodeStatuses: Record<string, NodeStatus>
    setNodeStatus: (nodeId: string, status: NodeStatus) => void
    resetNodeStatuses: () => void
    humanGate: HumanGateState | null
    setHumanGate: (gate: HumanGateState | null) => void
    clearHumanGate: () => void
}

export interface EditorSlice {
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
    markSaveInFlight: () => void
    markSaveSuccess: () => void
    markSaveConflict: (message: string) => void
    markSaveFailure: (message: string, kind?: SaveErrorKind) => void
    resetSaveState: () => void
}

export type AppState = WorkspaceSlice & RunInspectorSlice & EditorSlice
