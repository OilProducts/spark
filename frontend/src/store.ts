import { create } from 'zustand'

export type ViewMode = 'editor' | 'execution'
export type NodeStatus = 'idle' | 'running' | 'success' | 'failed' | 'waiting'
export type DiagnosticSeverity = 'error' | 'warning' | 'info'

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
    activeFlow: string | null
    setActiveFlow: (flow: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void
    selectedEdgeId: string | null
    setSelectedEdgeId: (id: string | null) => void

    logs: LogEntry[]
    addLog: (entry: LogEntry) => void
    clearLogs: () => void

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
}

export const useStore = create<AppState>((set) => ({
    viewMode: 'editor',
    setViewMode: (mode) => set({ viewMode: mode }),
    activeFlow: null,
    setActiveFlow: (flow) => set({ activeFlow: flow }),
    selectedNodeId: null,
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),
    selectedEdgeId: null,
    setSelectedEdgeId: (id) => set({ selectedEdgeId: id }),

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
}))
