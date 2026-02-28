import { create } from 'zustand'

export type ViewMode = 'projects' | 'editor' | 'execution' | 'settings' | 'runs'
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
export type SaveState = 'idle' | 'saving' | 'saved' | 'error'

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
    goal?: string
    label?: string
    model_stylesheet?: string
    default_max_retry?: number | string
    retry_target?: string
    fallback_retry_target?: string
    default_fidelity?: string
    ui_default_llm_model?: string
    ui_default_llm_provider?: string
    ui_default_reasoning_effort?: string
}

export interface RegisteredProject {
    directoryPath: string
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

const buildDiagnosticMaps = (diagnostics: DiagnosticEntry[]) => {
    const nodeDiagnostics: Record<string, DiagnosticEntry[]> = {}
    const edgeDiagnostics: Record<string, DiagnosticEntry[]> = {}

    diagnostics.forEach((diag) => {
        if (diag.node_id) {
            if (!nodeDiagnostics[diag.node_id]) {
                nodeDiagnostics[diag.node_id] = []
            }
            nodeDiagnostics[diag.node_id].push(diag)
        }
        if (diag.edge && diag.edge.length === 2) {
            const key = `${diag.edge[0]}->${diag.edge[1]}`
            if (!edgeDiagnostics[key]) {
                edgeDiagnostics[key] = []
            }
            edgeDiagnostics[key].push(diag)
        }
    })

    return { nodeDiagnostics, edgeDiagnostics }
}

export interface UiDefaults {
    llm_model: string
    llm_provider: string
    reasoning_effort: string
}

const DEFAULT_UI_DEFAULTS: UiDefaults = {
    llm_model: "",
    llm_provider: "",
    reasoning_effort: "",
}

const UI_DEFAULTS_STORAGE_KEY = "sparkspawn.ui_defaults"
const ROUTE_STATE_STORAGE_KEY = "sparkspawn.ui_route_state"
const VIEW_MODES: ViewMode[] = ['projects', 'editor', 'execution', 'settings', 'runs']
const modeRequiresActiveProject = (mode: ViewMode) => mode === 'editor' || mode === 'execution'
const resolveViewModeForProjectScope = (mode: ViewMode, activeProjectPath: string | null): ViewMode => {
    return modeRequiresActiveProject(mode) && !activeProjectPath ? 'projects' : mode
}
const normalizeProjectPath = (path: string) => {
    const trimmed = path.trim()
    if (!trimmed) return ""
    const slashNormalized = trimmed.replace(/\\/g, "/").replace(/\/{2,}/g, "/")
    const stripped = slashNormalized.replace(/\/+$/, "")
    return stripped || slashNormalized
}

interface RouteState {
    viewMode: ViewMode
    activeProjectPath: string | null
    activeFlow: string | null
    selectedRunId: string | null
}

const DEFAULT_ROUTE_STATE: RouteState = {
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    selectedRunId: null,
}

const loadRouteState = (): RouteState => {
    if (typeof window === "undefined") {
        return { ...DEFAULT_ROUTE_STATE }
    }
    try {
        const raw = window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY)
        if (!raw) return { ...DEFAULT_ROUTE_STATE }
        const parsed = JSON.parse(raw) as Partial<RouteState>
        const isValidViewMode = parsed.viewMode ? VIEW_MODES.includes(parsed.viewMode) : false
        const requestedViewMode = isValidViewMode ? parsed.viewMode! : DEFAULT_ROUTE_STATE.viewMode
        const parsedRouteState: RouteState = {
            viewMode: requestedViewMode,
            activeProjectPath: typeof parsed.activeProjectPath === "string" ? parsed.activeProjectPath : null,
            activeFlow: typeof parsed.activeFlow === "string" ? parsed.activeFlow : null,
            selectedRunId: typeof parsed.selectedRunId === "string" ? parsed.selectedRunId : null,
        }
        return {
            ...parsedRouteState,
            viewMode: resolveViewModeForProjectScope(parsedRouteState.viewMode, parsedRouteState.activeProjectPath),
        }
    } catch {
        return { ...DEFAULT_ROUTE_STATE }
    }
}

const saveRouteState = (state: RouteState) => {
    if (typeof window === "undefined") return
    try {
        window.localStorage.setItem(ROUTE_STATE_STORAGE_KEY, JSON.stringify(state))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
}

const loadUiDefaults = (): UiDefaults => {
    if (typeof window === "undefined") {
        return { ...DEFAULT_UI_DEFAULTS }
    }
    try {
        const raw = window.localStorage.getItem(UI_DEFAULTS_STORAGE_KEY)
        if (!raw) return { ...DEFAULT_UI_DEFAULTS }
        const parsed = JSON.parse(raw) as Partial<UiDefaults>
        return {
            ...DEFAULT_UI_DEFAULTS,
            ...parsed,
        }
    } catch {
        return { ...DEFAULT_UI_DEFAULTS }
    }
}

const saveUiDefaults = (defaults: UiDefaults) => {
    if (typeof window === "undefined") return
    try {
        window.localStorage.setItem(UI_DEFAULTS_STORAGE_KEY, JSON.stringify(defaults))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
}

interface AppState {
    viewMode: ViewMode
    setViewMode: (mode: ViewMode) => void
    activeProjectPath: string | null
    setActiveProjectPath: (projectPath: string | null) => void
    projectRegistry: Record<string, RegisteredProject>
    projectRegistrationError: string | null
    registerProject: (directoryPath: string) => ProjectRegistrationResult
    clearProjectRegistrationError: () => void
    activeFlow: string | null
    setActiveFlow: (flow: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void
    selectedEdgeId: string | null
    setSelectedEdgeId: (id: string | null) => void
    selectedRunId: string | null
    setSelectedRunId: (id: string | null) => void

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

    workingDir: string
    setWorkingDir: (value: string) => void
    model: string
    setModel: (value: string) => void

    graphAttrs: GraphAttrs
    setGraphAttrs: (attrs: GraphAttrs) => void
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
    saveErrorMessage: string | null
    markSaveInFlight: () => void
    markSaveSuccess: () => void
    markSaveFailure: (message: string) => void
}

const restoredRouteState = loadRouteState()

export const useStore = create<AppState>((set) => ({
    viewMode: restoredRouteState.viewMode,
    setViewMode: (mode) =>
        set((state) => {
            const nextViewMode = resolveViewModeForProjectScope(mode, state.activeProjectPath)
            saveRouteState({
                viewMode: nextViewMode,
                activeProjectPath: state.activeProjectPath,
                activeFlow: state.activeFlow,
                selectedRunId: state.selectedRunId,
            })
            return { viewMode: nextViewMode }
        }),
    activeProjectPath: restoredRouteState.activeProjectPath,
    setActiveProjectPath: (projectPath) =>
        set((state) => {
            const nextViewMode = resolveViewModeForProjectScope(state.viewMode, projectPath)
            saveRouteState({
                viewMode: nextViewMode,
                activeProjectPath: projectPath,
                activeFlow: state.activeFlow,
                selectedRunId: state.selectedRunId,
            })
            return {
                activeProjectPath: projectPath,
                viewMode: nextViewMode,
            }
        }),
    projectRegistry: {},
    projectRegistrationError: null,
    registerProject: (directoryPath) => {
        let result: ProjectRegistrationResult = {
            ok: false,
            error: 'Project directory path is required.',
        }
        set((state) => {
            const normalizedPath = normalizeProjectPath(directoryPath)
            if (!normalizedPath) {
                result = {
                    ok: false,
                    error: 'Project directory path is required.',
                }
                return { projectRegistrationError: result.error }
            }

            const duplicate = Boolean(state.projectRegistry[normalizedPath])
            if (duplicate) {
                result = {
                    ok: false,
                    normalizedPath,
                    error: `Project already registered: ${normalizedPath}`,
                }
                return { projectRegistrationError: result.error }
            }

            const nextActiveProjectPath = state.activeProjectPath ?? normalizedPath
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextActiveProjectPath,
                activeFlow: state.activeFlow,
                selectedRunId: state.selectedRunId,
            })
            result = {
                ok: true,
                normalizedPath,
            }
            return {
                projectRegistry: {
                    ...state.projectRegistry,
                    [normalizedPath]: { directoryPath: normalizedPath },
                },
                projectRegistrationError: null,
                activeProjectPath: nextActiveProjectPath,
            }
        })
        return result
    },
    clearProjectRegistrationError: () => set({ projectRegistrationError: null }),
    activeFlow: restoredRouteState.activeFlow,
    setActiveFlow: (flow) =>
        set((state) => {
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: state.activeProjectPath,
                activeFlow: flow,
                selectedRunId: state.selectedRunId,
            })
            return { activeFlow: flow }
        }),
    selectedNodeId: null,
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),
    selectedEdgeId: null,
    setSelectedEdgeId: (id) => set({ selectedEdgeId: id }),
    selectedRunId: restoredRouteState.selectedRunId,
    setSelectedRunId: (id) =>
        set((state) => {
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: state.activeProjectPath,
                activeFlow: state.activeFlow,
                selectedRunId: id,
            })
            return { selectedRunId: id }
        }),

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

    workingDir: "./test-app",
    setWorkingDir: (value) => set({ workingDir: value }),
    model: "",
    setModel: (value) => set({ model: value }),

    graphAttrs: {},
    setGraphAttrs: (attrs) => set({ graphAttrs: attrs }),
    updateGraphAttr: (key, value) =>
        set((state) => ({
            graphAttrs: {
                ...state.graphAttrs,
                [key]: value,
            },
        })),

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
    saveErrorMessage: null,
    markSaveInFlight: () => set({ saveState: 'saving', saveErrorMessage: null }),
    markSaveSuccess: () => set({ saveState: 'saved', saveErrorMessage: null }),
    markSaveFailure: (message) =>
        set({
            saveState: 'error',
            saveErrorMessage: message || 'Flow save failed.',
        }),
}))
