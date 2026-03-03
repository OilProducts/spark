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
    goal?: string
    label?: string
    model_stylesheet?: string
    default_max_retry?: number | string
    retry_target?: string
    fallback_retry_target?: string
    default_fidelity?: string
    'stack.child_dotfile'?: string
    'stack.child_workdir'?: string
    'tool_hooks.pre'?: string
    'tool_hooks.post'?: string
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

export interface ConversationHistoryEntry {
    role: 'user' | 'assistant' | 'system'
    content: string
    timestamp: string
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
const PROJECT_CONVERSATION_STATE_STORAGE_KEY = "sparkspawn.project_conversation_state"
const DEFAULT_WORKING_DIRECTORY = "./test-app"
const RECENT_PROJECT_LIMIT = 5
const VIEW_MODES: ViewMode[] = ['projects', 'editor', 'execution', 'settings', 'runs']
const modeRequiresActiveProject = (mode: ViewMode) => mode === 'editor' || mode === 'execution'
const resolveViewModeForProjectScope = (mode: ViewMode, activeProjectPath: string | null): ViewMode => {
    return modeRequiresActiveProject(mode) && !activeProjectPath ? 'projects' : mode
}
const normalizeProjectPath = (path: string) => {
    const trimmed = path.trim()
    if (!trimmed) return ""
    const slashNormalized = trimmed.replace(/\\/g, "/").replace(/\/{2,}/g, "/")
    const windowsPrefixMatch = slashNormalized.match(/^[A-Za-z]:\//)
    const prefix = slashNormalized.startsWith("/") ? "/" : windowsPrefixMatch ? windowsPrefixMatch[0] : ""
    const rawBody = prefix ? slashNormalized.slice(prefix.length) : slashNormalized
    const parts = rawBody.split("/").filter((part) => part.length > 0)
    const segments: string[] = []
    for (const part of parts) {
        if (part === ".") {
            continue
        }
        if (part === "..") {
            if (segments.length > 0) {
                segments.pop()
            }
            continue
        }
        segments.push(part)
    }
    const normalizedBody = segments.join("/")
    if (!normalizedBody) {
        if (prefix === "/") {
            return "/"
        }
        return prefix || normalizedBody
    }
    return `${prefix}${normalizedBody}`
}
const isAbsoluteProjectPath = (path: string) => path.startsWith("/") || /^[A-Za-z]:\//.test(path)
const pushRecentProjectPath = (recentProjectPaths: string[], projectPath: string | null) => {
    if (!projectPath) {
        return recentProjectPaths
    }
    const deduped = [projectPath, ...recentProjectPaths.filter((path) => path !== projectPath)]
    return deduped.slice(0, RECENT_PROJECT_LIMIT)
}

const GRAPH_FIDELITY_OPTION_SET = new Set<string>([
    'full',
    'truncate',
    'compact',
    'summary:low',
    'summary:medium',
    'summary:high',
])

const STRING_GRAPH_ATTR_KEYS: (keyof GraphAttrs)[] = [
    'goal',
    'label',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool_hooks.pre',
    'tool_hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_reasoning_effort',
]

const normalizeGraphAttrValue = (key: keyof GraphAttrs, value: string): string => {
    if (key === 'model_stylesheet') {
        return value
    }
    if (key === 'default_max_retry') {
        const trimmed = value.trim()
        if (!trimmed) return ''
        if (!/^\d+$/.test(trimmed)) return trimmed
        return `${Math.max(0, parseInt(trimmed, 10))}`
    }
    if (key === 'default_fidelity') {
        return value.trim().toLowerCase()
    }
    if (STRING_GRAPH_ATTR_KEYS.includes(key)) {
        return value.trim()
    }
    return value
}

const validateGraphAttrValue = (key: keyof GraphAttrs, value: string): string | null => {
    if (key === 'default_max_retry') {
        if (!value) return null
        if (!/^\d+$/.test(value)) {
            return 'Default max retry must be a non-negative integer.'
        }
        return null
    }
    if (key === 'default_fidelity') {
        if (!value) return null
        if (!GRAPH_FIDELITY_OPTION_SET.has(value)) {
            return 'Default fidelity must be one of: full, truncate, compact, summary:low, summary:medium, summary:high.'
        }
        return null
    }
    return null
}

const normalizeGraphAttrs = (attrs: GraphAttrs): GraphAttrs => {
    const normalized: GraphAttrs = {}
    const entries = Object.entries(attrs) as [keyof GraphAttrs, GraphAttrs[keyof GraphAttrs]][]
    entries.forEach(([key, rawValue]) => {
        if (rawValue === undefined || rawValue === null) {
            return
        }
        const value = normalizeGraphAttrValue(key, String(rawValue))
        normalized[key] = value
    })
    return normalized
}

const deriveGraphAttrErrors = (attrs: GraphAttrs): GraphAttrErrors => {
    const errors: GraphAttrErrors = {}
    const entries = Object.entries(attrs) as [keyof GraphAttrs, GraphAttrs[keyof GraphAttrs]][]
    entries.forEach(([key, rawValue]) => {
        const value = rawValue === undefined || rawValue === null ? '' : String(rawValue)
        const normalizedValue = normalizeGraphAttrValue(key, value)
        const error = validateGraphAttrValue(key, normalizedValue)
        if (error) {
            errors[key] = error
        }
    })
    return errors
}

interface RouteState {
    viewMode: ViewMode
    activeProjectPath: string | null
    activeFlow: string | null
    selectedRunId: string | null
}

interface ProjectScopedWorkspace {
    activeFlow: string | null
    selectedRunId: string | null
    workingDir: string
    conversationId: string | null
    conversationHistory: ConversationHistoryEntry[]
    specId: string | null
    specStatus: 'draft' | 'approved'
    planId: string | null
    planStatus: PlanStatus
    artifactRunId: string | null
}

type PersistedProjectConversationState = Record<
    string,
    {
        conversationId: string | null
        conversationHistory: ConversationHistoryEntry[]
    }
>

const DEFAULT_PROJECT_SCOPED_WORKSPACE: ProjectScopedWorkspace = {
    activeFlow: null,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    conversationId: null,
    conversationHistory: [],
    specId: null,
    specStatus: 'draft',
    planId: null,
    planStatus: 'draft',
    artifactRunId: null,
}

const coerceConversationHistoryEntry = (value: unknown): ConversationHistoryEntry | null => {
    if (!value || typeof value !== "object") {
        return null
    }
    const candidate = value as Partial<ConversationHistoryEntry>
    if (
        (candidate.role === "user" || candidate.role === "assistant" || candidate.role === "system")
        && typeof candidate.content === "string"
        && typeof candidate.timestamp === "string"
    ) {
        return {
            role: candidate.role,
            content: candidate.content,
            timestamp: candidate.timestamp,
        }
    }
    return null
}

const loadProjectConversationState = (): PersistedProjectConversationState => {
    if (typeof window === "undefined") {
        return {}
    }
    try {
        const raw = window.localStorage.getItem(PROJECT_CONVERSATION_STATE_STORAGE_KEY)
        if (!raw) {
            return {}
        }
        const parsed = JSON.parse(raw) as Record<string, unknown>
        const restored: PersistedProjectConversationState = {}
        Object.entries(parsed).forEach(([projectPath, value]) => {
            if (!value || typeof value !== "object") {
                return
            }
            const scope = value as {
                conversationId?: unknown
                conversationHistory?: unknown
            }
            const history = Array.isArray(scope.conversationHistory)
                ? scope.conversationHistory
                    .map(coerceConversationHistoryEntry)
                    .filter((entry): entry is ConversationHistoryEntry => entry !== null)
                : []
            restored[projectPath] = {
                conversationId: typeof scope.conversationId === "string" ? scope.conversationId : null,
                conversationHistory: history,
            }
        })
        return restored
    } catch {
        return {}
    }
}

const saveProjectConversationState = (projectScopedWorkspaces: Record<string, ProjectScopedWorkspace>) => {
    if (typeof window === "undefined") {
        return
    }
    try {
        const persisted: PersistedProjectConversationState = {}
        Object.entries(projectScopedWorkspaces).forEach(([projectPath, workspace]) => {
            persisted[projectPath] = {
                conversationId: workspace.conversationId,
                conversationHistory: workspace.conversationHistory,
            }
        })
        window.localStorage.setItem(PROJECT_CONVERSATION_STATE_STORAGE_KEY, JSON.stringify(persisted))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
}

const restoredProjectConversationState = loadProjectConversationState()

const resolveProjectScopedWorkspace = (
    workspace: Partial<ProjectScopedWorkspace> | undefined,
    projectPath: string | null
): ProjectScopedWorkspace => {
    const defaultWorkingDir = projectPath || DEFAULT_WORKING_DIRECTORY
    const restoredConversationState = projectPath ? restoredProjectConversationState[projectPath] : undefined
    return {
        ...DEFAULT_PROJECT_SCOPED_WORKSPACE,
        ...restoredConversationState,
        ...workspace,
        workingDir: workspace?.workingDir || defaultWorkingDir,
    }
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
    recentProjectPaths: string[]
    projectScopedWorkspaces: Record<string, ProjectScopedWorkspace>
    projectRegistrationError: string | null
    registerProject: (directoryPath: string) => ProjectRegistrationResult
    updateProjectPath: (currentDirectoryPath: string, nextDirectoryPath: string) => ProjectRegistrationResult
    toggleProjectFavorite: (projectPath: string) => void
    clearProjectRegistrationError: () => void
    activeFlow: string | null
    setActiveFlow: (flow: string | null) => void
    selectedNodeId: string | null
    setSelectedNodeId: (id: string | null) => void
    selectedEdgeId: string | null
    setSelectedEdgeId: (id: string | null) => void
    selectedRunId: string | null
    setSelectedRunId: (id: string | null) => void
    setConversationId: (id: string | null) => void
    appendConversationHistoryEntry: (entry: ConversationHistoryEntry) => void
    setSpecId: (id: string | null) => void
    setSpecStatus: (status: 'draft' | 'approved') => void
    setPlanId: (id: string | null) => void
    setPlanStatus: (status: PlanStatus) => void

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
    graphAttrErrors: GraphAttrErrors
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
    saveErrorKind: SaveErrorKind | null
    markSaveInFlight: () => void
    markSaveSuccess: () => void
    markSaveConflict: (message: string) => void
    markSaveFailure: (message: string, kind?: SaveErrorKind) => void
}

const restoredRouteState = loadRouteState()
const initialProjectScopedWorkspaces: Record<string, ProjectScopedWorkspace> = restoredRouteState.activeProjectPath
    ? {
        [restoredRouteState.activeProjectPath]: resolveProjectScopedWorkspace(
            {
                activeFlow: restoredRouteState.activeFlow,
                selectedRunId: restoredRouteState.selectedRunId,
            },
            restoredRouteState.activeProjectPath
        ),
    }
    : {}
const restoredProjectScope = restoredRouteState.activeProjectPath
    ? resolveProjectScopedWorkspace(
        initialProjectScopedWorkspaces[restoredRouteState.activeProjectPath],
        restoredRouteState.activeProjectPath
    )
    : null

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
    projectScopedWorkspaces: initialProjectScopedWorkspaces,
    recentProjectPaths: restoredRouteState.activeProjectPath ? [restoredRouteState.activeProjectPath] : [],
    setActiveProjectPath: (projectPath) =>
        set((state) => {
            const isProjectSwitch = projectPath !== state.activeProjectPath
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const nextProjectRegistry = { ...state.projectRegistry }
            const currentScope = state.activeProjectPath
            if (currentScope) {
                const existingScope = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[currentScope], currentScope)
                nextProjectScopedWorkspaces[currentScope] = {
                    ...existingScope,
                    activeFlow: state.activeFlow,
                    selectedRunId: state.selectedRunId,
                    workingDir: state.workingDir,
                }
            }
            const nextProjectScope = projectPath ? resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[projectPath], projectPath) : null
            if (projectPath && nextProjectScope) {
                nextProjectScopedWorkspaces[projectPath] = nextProjectScope
            }
            if (projectPath && nextProjectRegistry[projectPath]) {
                nextProjectRegistry[projectPath] = {
                    ...nextProjectRegistry[projectPath],
                    lastAccessedAt: new Date().toISOString(),
                }
            }
            const nextViewMode = resolveViewModeForProjectScope(state.viewMode, projectPath)
            saveRouteState({
                viewMode: nextViewMode,
                activeProjectPath: projectPath,
                activeFlow: projectPath ? nextProjectScope.activeFlow : null,
                selectedRunId: projectPath ? nextProjectScope.selectedRunId : null,
            })
            return {
                activeProjectPath: projectPath,
                viewMode: nextViewMode,
                projectRegistry: nextProjectRegistry,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
                recentProjectPaths: pushRecentProjectPath(state.recentProjectPaths, projectPath),
                activeFlow: projectPath ? nextProjectScope.activeFlow : null,
                selectedRunId: projectPath ? nextProjectScope.selectedRunId : null,
                workingDir: projectPath ? nextProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
                runtimeStatus: isProjectSwitch ? 'idle' : state.runtimeStatus,
                nodeStatuses: isProjectSwitch ? {} : state.nodeStatuses,
                humanGate: isProjectSwitch ? null : state.humanGate,
                logs: isProjectSwitch ? [] : state.logs,
                selectedNodeId: isProjectSwitch ? null : state.selectedNodeId,
                selectedEdgeId: isProjectSwitch ? null : state.selectedEdgeId,
                graphAttrs: isProjectSwitch ? {} : state.graphAttrs,
                graphAttrErrors: isProjectSwitch ? {} : state.graphAttrErrors,
                diagnostics: isProjectSwitch ? [] : state.diagnostics,
                nodeDiagnostics: isProjectSwitch ? {} : state.nodeDiagnostics,
                edgeDiagnostics: isProjectSwitch ? {} : state.edgeDiagnostics,
                hasValidationErrors: isProjectSwitch ? false : state.hasValidationErrors,
                saveState: isProjectSwitch ? 'idle' : state.saveState,
                saveErrorMessage: isProjectSwitch ? null : state.saveErrorMessage,
                saveErrorKind: isProjectSwitch ? null : state.saveErrorKind,
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
            if (!isAbsoluteProjectPath(normalizedPath)) {
                result = {
                    ok: false,
                    normalizedPath,
                    error: 'Project directory path must be absolute.',
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
            const shouldActivateNewProject = !state.activeProjectPath
            const nowIso = shouldActivateNewProject ? new Date().toISOString() : null
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            nextProjectScopedWorkspaces[normalizedPath] = resolveProjectScopedWorkspace(
                nextProjectScopedWorkspaces[normalizedPath],
                normalizedPath
            )
            const nextActiveProjectScope = resolveProjectScopedWorkspace(
                nextProjectScopedWorkspaces[nextActiveProjectPath],
                nextActiveProjectPath
            )
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextActiveProjectPath,
                activeFlow: state.activeProjectPath ? state.activeFlow : nextActiveProjectScope.activeFlow,
                selectedRunId: state.activeProjectPath ? state.selectedRunId : nextActiveProjectScope.selectedRunId,
            })
            result = {
                ok: true,
                normalizedPath,
            }
            return {
                projectRegistry: {
                    ...state.projectRegistry,
                    // [normalizedPath]: { directoryPath: normalizedPath },
                    [normalizedPath]: {
                        directoryPath: normalizedPath,
                        isFavorite: false,
                        lastAccessedAt: nowIso,
                    },
                },
                recentProjectPaths: shouldActivateNewProject
                    ? pushRecentProjectPath(state.recentProjectPaths, normalizedPath)
                    : state.recentProjectPaths,
                projectRegistrationError: null,
                activeProjectPath: nextActiveProjectPath,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
                activeFlow: state.activeProjectPath ? state.activeFlow : nextActiveProjectScope.activeFlow,
                selectedRunId: state.activeProjectPath ? state.selectedRunId : nextActiveProjectScope.selectedRunId,
                workingDir: state.activeProjectPath ? state.workingDir : nextActiveProjectScope.workingDir,
            }
        })
        return result
    },
    updateProjectPath: (currentDirectoryPath, nextDirectoryPath) => {
        let result: ProjectRegistrationResult = {
            ok: false,
            error: 'Project directory path is required.',
        }
        set((state) => {
            const normalizedCurrentPath = normalizeProjectPath(currentDirectoryPath)
            const normalizedNextPath = normalizeProjectPath(nextDirectoryPath)

            if (!normalizedCurrentPath || !state.projectRegistry[normalizedCurrentPath]) {
                result = {
                    ok: false,
                    error: 'Project must already be registered before updating path.',
                }
                return { projectRegistrationError: result.error }
            }
            if (!normalizedNextPath) {
                result = {
                    ok: false,
                    error: 'Project directory path is required.',
                }
                return { projectRegistrationError: result.error }
            }
            if (!isAbsoluteProjectPath(normalizedNextPath)) {
                result = {
                    ok: false,
                    normalizedPath: normalizedNextPath,
                    error: 'Project directory path must be absolute.',
                }
                return { projectRegistrationError: result.error }
            }
            const duplicate = normalizedNextPath !== normalizedCurrentPath && Boolean(state.projectRegistry[normalizedNextPath])
            if (duplicate) {
                result = {
                    ok: false,
                    normalizedPath: normalizedNextPath,
                    error: `Project already registered: ${normalizedNextPath}`,
                }
                return { projectRegistrationError: result.error }
            }
            if (normalizedNextPath === normalizedCurrentPath) {
                result = {
                    ok: true,
                    normalizedPath: normalizedCurrentPath,
                }
                return { projectRegistrationError: null }
            }

            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const currentWorkspace = resolveProjectScopedWorkspace(
                nextProjectScopedWorkspaces[normalizedCurrentPath],
                normalizedCurrentPath
            )
            delete nextProjectScopedWorkspaces[normalizedCurrentPath]
            nextProjectScopedWorkspaces[normalizedNextPath] = {
                ...currentWorkspace,
                workingDir: currentWorkspace.workingDir === normalizedCurrentPath
                    ? normalizedNextPath
                    : currentWorkspace.workingDir,
            }

            const nextProjectRegistry = { ...state.projectRegistry }
            delete nextProjectRegistry[normalizedCurrentPath]
            nextProjectRegistry[normalizedNextPath] = {
                ...state.projectRegistry[normalizedCurrentPath],
                directoryPath: normalizedNextPath,
            }
            const activeProjectWasUpdated = state.activeProjectPath === normalizedCurrentPath
            const nextActiveProjectPath = activeProjectWasUpdated ? normalizedNextPath : state.activeProjectPath
            const nextWorkingDir = activeProjectWasUpdated && state.workingDir === normalizedCurrentPath
                ? normalizedNextPath
                : state.workingDir
            const nextRecentProjectPaths = state.recentProjectPaths.map((path) =>
                path === normalizedCurrentPath ? normalizedNextPath : path
            )

            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: nextActiveProjectPath,
                activeFlow: state.activeFlow,
                selectedRunId: state.selectedRunId,
            })
            saveProjectConversationState(nextProjectScopedWorkspaces)
            result = {
                ok: true,
                normalizedPath: normalizedNextPath,
            }
            return {
                projectRegistry: nextProjectRegistry,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
                activeProjectPath: nextActiveProjectPath,
                workingDir: nextWorkingDir,
                recentProjectPaths: nextRecentProjectPaths,
                projectRegistrationError: null,
            }
        })
        return result
    },
    toggleProjectFavorite: (projectPath) =>
        set((state) => {
            const normalizedPath = normalizeProjectPath(projectPath)
            const project = state.projectRegistry[normalizedPath]
            if (!project) {
                return state
            }
            return {
                projectRegistry: {
                    ...state.projectRegistry,
                    [normalizedPath]: {
                        ...project,
                        isFavorite: !project.isFavorite,
                    },
                },
            }
        }),
    clearProjectRegistrationError: () => set({ projectRegistrationError: null }),
    activeFlow: restoredProjectScope ? restoredProjectScope.activeFlow : restoredRouteState.activeFlow,
    setActiveFlow: (flow) =>
        set((state) => {
            if (!state.activeProjectPath) {
                saveRouteState({
                    viewMode: state.viewMode,
                    activeProjectPath: null,
                    activeFlow: null,
                    selectedRunId: state.selectedRunId,
                })
                return { activeFlow: null }
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                activeFlow: flow,
            }
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: state.activeProjectPath,
                activeFlow: flow,
                selectedRunId: state.selectedRunId,
            })
            return {
                activeFlow: flow,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    selectedNodeId: null,
    setSelectedNodeId: (id) => set({ selectedNodeId: id }),
    selectedEdgeId: null,
    setSelectedEdgeId: (id) => set({ selectedEdgeId: id }),
    selectedRunId: restoredProjectScope ? restoredProjectScope.selectedRunId : restoredRouteState.selectedRunId,
    setSelectedRunId: (id) =>
        set((state) => {
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            if (state.activeProjectPath) {
                const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
                nextProjectScopedWorkspaces[state.activeProjectPath] = {
                    ...scoped,
                    selectedRunId: id,
                    artifactRunId: id,
                }
            }
            saveRouteState({
                viewMode: state.viewMode,
                activeProjectPath: state.activeProjectPath,
                activeFlow: state.activeFlow,
                selectedRunId: id,
            })
            return {
                selectedRunId: id,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    setConversationId: (id) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                conversationId: id,
            }
            saveProjectConversationState(nextProjectScopedWorkspaces)
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    appendConversationHistoryEntry: (entry) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                conversationHistory: [...scoped.conversationHistory, entry],
            }
            saveProjectConversationState(nextProjectScopedWorkspaces)
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    setSpecId: (id) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                specId: id,
            }
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    setSpecStatus: (status) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                specStatus: status,
            }
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    setPlanId: (id) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                planId: id,
            }
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    setPlanStatus: (status) =>
        set((state) => {
            if (!state.activeProjectPath) {
                return {}
            }
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
            nextProjectScopedWorkspaces[state.activeProjectPath] = {
                ...scoped,
                planStatus: status,
            }
            return {
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
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

    workingDir: restoredProjectScope ? restoredProjectScope.workingDir : DEFAULT_WORKING_DIRECTORY,
    setWorkingDir: (value) =>
        set((state) => {
            const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
            if (state.activeProjectPath) {
                const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)
                nextProjectScopedWorkspaces[state.activeProjectPath] = {
                    ...scoped,
                    workingDir: value,
                }
            }
            return {
                workingDir: value,
                projectScopedWorkspaces: nextProjectScopedWorkspaces,
            }
        }),
    model: "",
    setModel: (value) => set({ model: value }),

    graphAttrs: {},
    graphAttrErrors: {},
    setGraphAttrs: (attrs) => {
        const normalizedAttrs = normalizeGraphAttrs(attrs)
        set({
            graphAttrs: normalizedAttrs,
            graphAttrErrors: deriveGraphAttrErrors(normalizedAttrs),
        })
    },
    updateGraphAttr: (key, value) =>
        set((state) => {
            const normalizedValue = normalizeGraphAttrValue(key, value)
            const error = validateGraphAttrValue(key, normalizedValue)
            const graphAttrErrors = {
                ...state.graphAttrErrors,
            }
            if (error) {
                graphAttrErrors[key] = error
            } else {
                delete graphAttrErrors[key]
            }
            return {
                graphAttrs: {
                    ...state.graphAttrs,
                    [key]: normalizedValue,
                },
                graphAttrErrors,
            }
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
    saveErrorMessage: null,
    saveErrorKind: null,
    markSaveInFlight: () => set({ saveState: 'saving', saveErrorMessage: null, saveErrorKind: null }),
    markSaveSuccess: () => set({ saveState: 'saved', saveErrorMessage: null, saveErrorKind: null }),
    markSaveConflict: (message) =>
        set({
            saveState: 'conflict',
            saveErrorMessage: message || 'Flow save conflict detected.',
            saveErrorKind: 'conflict',
        }),
    markSaveFailure: (message, kind = 'unknown') =>
        set({
            saveState: 'error',
            saveErrorMessage: message || 'Flow save failed.',
            saveErrorKind: kind,
        }),
}))
